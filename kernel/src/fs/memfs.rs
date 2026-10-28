extern crate alloc;

use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use spin::Mutex;

use super::vfs::{
    self, DirCursor, DirEntry, FileSystem, FileType, FsError, FsStats, Inode, SetMetadata,
};
use crate::boot_settings::TmpfsOptions;
use crate::mm::page::{OwnedPage, PAGE_SIZE};

pub struct MemoryFs {
    root: Arc<MemoryInode>,
}

struct Shared {
    fs_id: u64,
    limits: Option<TmpfsOptions>,
    pages: AtomicUsize,
    inodes: AtomicUsize,
    namespace: Mutex<Vec<Weak<MemoryInode>>>,
}

#[derive(Clone, Copy)]
enum Resource {
    Page,
    Inode,
}

struct Charge {
    shared: Arc<Shared>,
    resource: Resource,
}

impl Shared {
    fn counter(&self, resource: Resource) -> (&AtomicUsize, usize) {
        match resource {
            Resource::Page => (
                &self.pages,
                self.limits.map_or(usize::MAX, |l| l.size / PAGE_SIZE),
            ),
            Resource::Inode => (
                &self.inodes,
                self.limits.map_or(usize::MAX, |l| l.nr_inodes),
            ),
        }
    }

    fn reserve(self: &Arc<Self>, resource: Resource) -> Result<Charge, FsError> {
        let (counter, limit) = self.counter(resource);
        counter
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                (used < limit).then(|| used + 1)
            })
            .map_err(|_| FsError::NoSpace)?;
        Ok(Charge {
            shared: self.clone(),
            resource,
        })
    }

    fn name(&self) -> &'static str {
        if self.limits.is_some() {
            "tmpfs"
        } else {
            "ramfs"
        }
    }
}

impl Drop for Charge {
    fn drop(&mut self) {
        self.shared
            .counter(self.resource)
            .0
            .fetch_sub(1, Ordering::AcqRel);
    }
}

impl MemoryFs {
    pub fn new() -> Self {
        Self::build(None).expect("cannot allocate ramfs root")
    }

    pub fn tmpfs(options: TmpfsOptions) -> Result<Self, FsError> {
        if options.size == 0
            || options.size % PAGE_SIZE != 0
            || options.nr_inodes == 0
            || options.mode > 0o7777
        {
            return Err(FsError::InvalidArgument);
        }
        Self::build(Some(options))
    }

    fn build(limits: Option<TmpfsOptions>) -> Result<Self, FsError> {
        let shared = Arc::try_new(Shared {
            fs_id: vfs::allocate_filesystem_id(),
            limits,
            pages: AtomicUsize::new(0),
            inodes: AtomicUsize::new(0),
            namespace: Mutex::new(Vec::new()),
        })
        .map_err(|_| FsError::OutOfMemory)?;
        let root = MemoryInode::new(&shared, FileType::Directory, 0)?;
        {
            let mut nodes = shared.namespace.lock();
            nodes.try_reserve(1).map_err(|_| FsError::OutOfMemory)?;
            nodes.push(Arc::downgrade(&root));
        }
        Ok(Self { root })
    }
}

impl Default for MemoryFs {
    fn default() -> Self {
        Self::new()
    }
}

impl FileSystem for MemoryFs {
    fn name(&self) -> &str {
        self.root.shared().name()
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }

    fn fs_id(&self) -> u64 {
        self.root.shared().fs_id
    }

    fn statfs(&self) -> Result<FsStats, FsError> {
        let shared = self.root.shared();
        let limits = shared.limits.ok_or(FsError::NotSupported)?;
        let pages = limits.size / PAGE_SIZE;
        Ok(FsStats {
            magic: 0x0102_1994,
            block_size: PAGE_SIZE as u64,
            blocks: pages as u64,
            blocks_free: pages.saturating_sub(shared.pages.load(Ordering::Acquire)) as u64,
            files: limits.nr_inodes as u64,
            files_free: limits
                .nr_inodes
                .saturating_sub(shared.inodes.load(Ordering::Acquire))
                as u64,
            name_max: 255,
        })
    }
}

pub struct MemoryInode {
    ino: u64,
    parent: AtomicU64,
    links: AtomicU64,
    file_type: FileType,
    charge: Charge,
    inner: Mutex<InodeData>,
}

struct InodeData {
    uid: u32,
    gid: u32,
    mode: u16,
    storage: Storage,
}

enum Storage {
    Directory(Vec<(String, Arc<MemoryInode>)>),
    Ram(Vec<u8>),
    Sparse { size: usize, pages: Vec<FilePage> },
}

struct FilePage {
    index: usize,
    data: OwnedPage,
    _charge: Charge,
}

impl MemoryInode {
    fn new(shared: &Arc<Shared>, file_type: FileType, parent: u64) -> Result<Arc<Self>, FsError> {
        let charge = shared.reserve(Resource::Inode)?;
        let options = shared.limits.unwrap_or_default();
        let storage = if file_type == FileType::Directory {
            Storage::Directory(Vec::new())
        } else if shared.limits.is_some() {
            Storage::Sparse {
                size: 0,
                pages: Vec::new(),
            }
        } else {
            Storage::Ram(Vec::new())
        };
        static NEXT_INO: AtomicU64 = AtomicU64::new(1);
        Arc::try_new(Self {
            ino: NEXT_INO.fetch_add(1, Ordering::Relaxed),
            parent: AtomicU64::new(parent),
            links: AtomicU64::new(if file_type == FileType::Directory {
                2
            } else {
                1
            }),
            file_type,
            charge,
            inner: Mutex::new(InodeData {
                uid: options.uid,
                gid: options.gid,
                mode: if parent == 0 {
                    options.mode
                } else if file_type == FileType::Directory {
                    0o755
                } else if file_type == FileType::Regular {
                    0o644
                } else {
                    0o777
                },
                storage,
            }),
        })
        .map_err(|_| FsError::OutOfMemory)
    }

    fn shared(&self) -> &Arc<Shared> {
        &self.charge.shared
    }

    fn validate_name(name: &str) -> Result<(), FsError> {
        if name.is_empty() || matches!(name, "." | "..") || name.contains(['/', '\0']) {
            return Err(FsError::InvalidArgument);
        }
        if name.len() > 255 {
            return Err(FsError::NameTooLong);
        }
        Ok(())
    }

    fn name_copy(name: &str) -> Result<String, FsError> {
        let mut copy = String::new();
        copy.try_reserve(name.len())
            .map_err(|_| FsError::OutOfMemory)?;
        copy.push_str(name);
        Ok(copy)
    }

    fn entries(data: &mut InodeData) -> Result<&mut Vec<(String, Arc<Self>)>, FsError> {
        match &mut data.storage {
            Storage::Directory(entries) => Ok(entries),
            _ => Err(FsError::NotADirectory),
        }
    }

    fn can_replace(source: &Self, target: &Self) -> Result<(), FsError> {
        match (source.file_type, target.file_type) {
            (FileType::Directory, FileType::Directory) => {
                let inner = target.inner.lock();
                if matches!(&inner.storage, Storage::Directory(entries) if !entries.is_empty()) {
                    return Err(FsError::NotEmpty);
                }
            }
            (FileType::Directory, _) => return Err(FsError::NotADirectory),
            (_, FileType::Directory) => return Err(FsError::IsADirectory),
            _ => {}
        }
        Ok(())
    }

    fn detach(&self, child: &Self) {
        child.links.store(0, Ordering::Release);
        if child.file_type == FileType::Directory {
            self.links.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

impl Inode for MemoryInode {
    fn ino(&self) -> u64 {
        self.ino
    }
    fn fs_id(&self) -> u64 {
        self.shared().fs_id
    }
    fn filesystem_name(&self) -> &'static str {
        self.shared().name()
    }
    fn file_type(&self) -> FileType {
        self.file_type
    }
    fn uid(&self) -> u32 {
        self.inner.lock().uid
    }
    fn gid(&self) -> u32 {
        self.inner.lock().gid
    }
    fn mode(&self) -> u16 {
        self.inner.lock().mode
    }
    fn links(&self) -> u64 {
        self.links.load(Ordering::Acquire)
    }

    fn set_metadata(&self, metadata: SetMetadata) -> Result<(), FsError> {
        if metadata.mode.is_some_and(|mode| mode > 0o7777) {
            return Err(FsError::InvalidArgument);
        }
        let mut inner = self.inner.lock();
        if let Some(uid) = metadata.uid {
            inner.uid = uid;
        }
        if let Some(gid) = metadata.gid {
            inner.gid = gid;
        }
        if let Some(mode) = metadata.mode {
            inner.mode = mode;
        }
        Ok(())
    }

    fn size(&self) -> usize {
        match &self.inner.lock().storage {
            Storage::Directory(entries) => entries.len(),
            Storage::Ram(data) => data.len(),
            Storage::Sparse { size, .. } => *size,
        }
    }

    fn blocks(&self) -> u64 {
        match &self.inner.lock().storage {
            Storage::Directory(_) => 0,
            Storage::Ram(data) => (data.len() as u64).div_ceil(512),
            Storage::Sparse { pages, .. } => (pages.len() * (PAGE_SIZE / 512)) as u64,
        }
    }

    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, FsError> {
        let inner = self.inner.lock();
        match &inner.storage {
            Storage::Directory(_) => Err(FsError::IsADirectory),
            Storage::Ram(data) => {
                let count = buf.len().min(data.len().saturating_sub(offset));
                if count > 0 {
                    buf[..count].copy_from_slice(&data[offset..offset + count]);
                }
                Ok(count)
            }
            Storage::Sparse { size, pages } => {
                let count = buf.len().min(size.saturating_sub(offset));
                buf[..count].fill(0);
                let mut copied = 0;
                while copied < count {
                    let position = offset + copied;
                    let start = position % PAGE_SIZE;
                    let len = (PAGE_SIZE - start).min(count - copied);
                    if let Ok(index) =
                        pages.binary_search_by_key(&(position / PAGE_SIZE), |p| p.index)
                    {
                        buf[copied..copied + len]
                            .copy_from_slice(&pages[index].data.bytes()[start..start + len]);
                    }
                    copied += len;
                }
                Ok(count)
            }
        }
    }

    fn write(&self, offset: usize, buf: &[u8]) -> Result<usize, FsError> {
        self.write_data(&mut self.inner.lock(), offset, buf)
    }

    fn append(&self, buf: &[u8]) -> Result<(usize, usize), FsError> {
        let mut inner = self.inner.lock();
        let offset = match &inner.storage {
            Storage::Directory(_) => return Err(FsError::IsADirectory),
            Storage::Ram(data) => data.len(),
            Storage::Sparse { size, .. } => *size,
        };
        let written = self.write_data(&mut inner, offset, buf)?;
        Ok((offset, written))
    }

    fn truncate(&self, new_size: usize) -> Result<(), FsError> {
        if new_size > isize::MAX as usize {
            return Err(FsError::FileTooLarge);
        }
        let mut inner = self.inner.lock();
        match &mut inner.storage {
            Storage::Directory(_) => Err(FsError::IsADirectory),
            Storage::Ram(data) => {
                if new_size > data.len() {
                    data.try_reserve(new_size - data.len())
                        .map_err(|_| FsError::OutOfMemory)?;
                }
                data.resize(new_size, 0);
                Ok(())
            }
            Storage::Sparse { size, pages } => {
                if new_size < *size {
                    pages.retain(|page| page.index < new_size.div_ceil(PAGE_SIZE));
                    if new_size % PAGE_SIZE != 0 {
                        if let Ok(slot) =
                            pages.binary_search_by_key(&(new_size / PAGE_SIZE), |p| p.index)
                        {
                            pages[slot].data.bytes_mut()[new_size % PAGE_SIZE..].fill(0);
                        }
                    }
                }
                *size = new_size;
                Ok(())
            }
        }
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>, FsError> {
        let _namespace = self.shared().namespace.lock();
        let mut inner = self.inner.lock();
        Self::entries(&mut inner)?
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, inode)| -> Arc<dyn Inode> { inode.clone() })
            .ok_or(FsError::NotFound)
    }

    fn create(&self, name: &str, file_type: FileType) -> Result<Arc<dyn Inode>, FsError> {
        Self::validate_name(name)?;
        if !matches!(
            file_type,
            FileType::Regular | FileType::Directory | FileType::Symlink
        ) {
            return Err(FsError::NotSupported);
        }
        let mut nodes = self.shared().namespace.lock();
        if self.links() == 0 {
            return Err(FsError::NotFound);
        }
        let mut inner = self.inner.lock();
        let entries = Self::entries(&mut inner)?;
        if entries.iter().any(|(n, _)| n == name) {
            return Err(FsError::AlreadyExists);
        }
        let name = Self::name_copy(name)?;
        entries.try_reserve(1).map_err(|_| FsError::OutOfMemory)?;
        nodes.retain(|node| node.strong_count() > 0);
        nodes.try_reserve(1).map_err(|_| FsError::OutOfMemory)?;
        let inode = Self::new(self.shared(), file_type, self.ino)?;
        nodes.push(Arc::downgrade(&inode));
        if file_type == FileType::Directory {
            self.links.fetch_add(1, Ordering::AcqRel);
        }
        entries.push((name, inode.clone()));
        Ok(inode)
    }

    fn unlink(&self, name: &str, directory: bool) -> Result<(), FsError> {
        Self::validate_name(name)?;
        let _namespace = self.shared().namespace.lock();
        let mut inner = self.inner.lock();
        let entries = Self::entries(&mut inner)?;
        let index = entries
            .iter()
            .position(|(n, _)| n == name)
            .ok_or(FsError::NotFound)?;
        let child = &entries[index].1;
        if child.file_type == FileType::Directory {
            if !directory {
                return Err(FsError::IsADirectory);
            }
            let inner = child.inner.lock();
            if matches!(&inner.storage, Storage::Directory(entries) if !entries.is_empty()) {
                return Err(FsError::NotEmpty);
            }
        } else if directory {
            return Err(FsError::NotADirectory);
        }
        self.detach(child);
        entries.remove(index);
        Ok(())
    }

    fn rename(&self, name: &str, destination: &dyn Inode, new_name: &str) -> Result<(), FsError> {
        Self::validate_name(name)?;
        Self::validate_name(new_name)?;
        if destination.fs_id() != self.fs_id() {
            return Err(FsError::CrossDevice);
        }
        let destination = (destination as &dyn core::any::Any)
            .downcast_ref::<Self>()
            .ok_or(FsError::NotSupported)?;
        let nodes = self.shared().namespace.lock();
        if destination.links() == 0 {
            return Err(FsError::NotFound);
        }
        let mut inner = self.inner.lock();
        let entries = Self::entries(&mut inner)?;
        let index = entries
            .iter()
            .position(|(n, _)| n == name)
            .ok_or(FsError::NotFound)?;
        let source = entries[index].1.clone();
        if self.ino == destination.ino && name == new_name {
            return Ok(());
        }
        if source.file_type == FileType::Directory {
            let mut ancestor = destination.ino;
            while ancestor != 0 {
                if ancestor == source.ino {
                    return Err(FsError::InvalidArgument);
                }
                ancestor = nodes
                    .iter()
                    .filter_map(Weak::upgrade)
                    .find(|node| node.ino == ancestor)
                    .ok_or(FsError::NotFound)?
                    .parent
                    .load(Ordering::Acquire);
            }
        }
        let new_name = Self::name_copy(new_name)?;
        if self.ino == destination.ino {
            if let Some(target) = entries.iter().position(|(n, _)| n == &new_name) {
                Self::can_replace(&source, &entries[target].1)?;
                self.detach(&entries[target].1);
                entries.remove(target);
            }
            let entry = entries
                .iter_mut()
                .find(|(_, node)| node.ino == source.ino)
                .ok_or(FsError::NotFound)?;
            entry.0 = new_name;
        } else {
            let mut destination_inner = destination.inner.lock();
            let destination_entries = Self::entries(&mut destination_inner)?;
            let target = destination_entries.iter().position(|(n, _)| n == &new_name);
            if let Some(target) = target {
                Self::can_replace(&source, &destination_entries[target].1)?;
            } else {
                destination_entries
                    .try_reserve(1)
                    .map_err(|_| FsError::OutOfMemory)?;
            }
            if let Some(target) = target {
                destination.detach(&destination_entries[target].1);
                destination_entries.remove(target);
            }
            entries.remove(index);
            if source.file_type == FileType::Directory {
                self.links.fetch_sub(1, Ordering::AcqRel);
                destination.links.fetch_add(1, Ordering::AcqRel);
            }
            source.parent.store(destination.ino, Ordering::Release);
            destination_entries.push((new_name, source));
        }
        Ok(())
    }

    fn readdir(
        &self,
        cursor: &mut DirCursor,
        _name_buf: &mut [u8],
        visit: &mut dyn for<'a> FnMut(DirEntry<'a>) -> bool,
    ) -> Result<usize, FsError> {
        let _namespace = self.shared().namespace.lock();
        let mut inner = self.inner.lock();
        let entries = Self::entries(&mut inner)?;
        let mut emitted = 0;
        let mut index = usize::try_from(cursor.offset).map_err(|_| FsError::InvalidArgument)?;
        while let Some((name, inode)) = entries.get(index) {
            let mut entry = DirEntry::new(name, inode.file_type, inode.ino);
            let metadata = inode.inner.lock();
            entry.uid = metadata.uid;
            entry.gid = metadata.gid;
            entry.mode = metadata.mode;
            drop(metadata);
            emitted += 1;
            index += 1;
            if !visit(entry) {
                break;
            }
        }
        cursor.offset = index as u64;
        Ok(emitted)
    }
}

impl MemoryInode {
    fn write_data(
        &self,
        inner: &mut InodeData,
        offset: usize,
        buf: &[u8],
    ) -> Result<usize, FsError> {
        if matches!(inner.storage, Storage::Directory(_)) {
            return Err(FsError::IsADirectory);
        }
        if buf.is_empty() {
            return Ok(0);
        }
        let end = offset
            .checked_add(buf.len())
            .filter(|n| *n <= isize::MAX as usize)
            .ok_or(FsError::FileTooLarge)?;
        match &mut inner.storage {
            Storage::Directory(_) => unreachable!(),
            Storage::Ram(data) => {
                if end > data.len() {
                    data.try_reserve(end - data.len())
                        .map_err(|_| FsError::OutOfMemory)?;
                    data.resize(end, 0);
                }
                data[offset..end].copy_from_slice(buf);
                Ok(buf.len())
            }
            Storage::Sparse { size, pages } => {
                let mut written = 0;
                while written < buf.len() {
                    let position = offset + written;
                    let page_index = position / PAGE_SIZE;
                    let slot = match pages.binary_search_by_key(&page_index, |p| p.index) {
                        Ok(slot) => slot,
                        Err(slot) => {
                            let allocate = || -> Result<FilePage, FsError> {
                                let charge = self.shared().reserve(Resource::Page)?;
                                let data = OwnedPage::new().ok_or(FsError::OutOfMemory)?;
                                Ok(FilePage {
                                    index: page_index,
                                    data,
                                    _charge: charge,
                                })
                            };
                            let result = pages
                                .try_reserve(1)
                                .map_err(|_| FsError::OutOfMemory)
                                .and_then(|_| allocate());
                            match result {
                                Ok(page) => pages.insert(slot, page),
                                Err(error) if written == 0 => return Err(error),
                                Err(_) => return Ok(written),
                            }
                            slot
                        }
                    };
                    let start = position % PAGE_SIZE;
                    let count = (PAGE_SIZE - start).min(buf.len() - written);
                    pages[slot].data.bytes_mut()[start..start + count]
                        .copy_from_slice(&buf[written..written + count]);
                    written += count;
                    *size = (*size).max(offset + written);
                }
                Ok(written)
            }
        }
    }
}
