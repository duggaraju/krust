extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use spin::Mutex;

use super::vfs::{DirCursor, DirEntry, FileSystem, FileType, FsError, Inode};

pub struct RamFs {
    fs_id: u64,
    root: Arc<RamInode>,
}

impl RamFs {
    pub fn new() -> Self {
        let fs_id = super::vfs::allocate_filesystem_id();
        Self {
            fs_id,
            root: Arc::new(RamInode::directory(fs_id)),
        }
    }
}

impl Default for RamFs {
    fn default() -> Self {
        Self::new()
    }
}

impl FileSystem for RamFs {
    fn name(&self) -> &str {
        "ramfs"
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }

    fn fs_id(&self) -> u64 {
        self.fs_id
    }
}

pub struct RamInode {
    fs_id: u64,
    ino: u64,
    file_type: FileType,
    inner: Mutex<RamInodeData>,
}

enum RamInodeData {
    File(Vec<u8>),
    Directory(Vec<(String, Arc<RamInode>)>),
}

impl RamInode {
    pub fn new(file_type: FileType, fs_id: u64) -> Self {
        let ino = next_inode_number();
        let inner = match file_type {
            FileType::Directory => RamInodeData::Directory(Vec::new()),
            _ => RamInodeData::File(Vec::new()),
        };

        Self {
            fs_id,
            ino,
            file_type,
            inner: Mutex::new(inner),
        }
    }

    pub fn directory(fs_id: u64) -> Self {
        Self::new(FileType::Directory, fs_id)
    }
}

impl Inode for RamInode {
    fn ino(&self) -> u64 {
        self.ino
    }

    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, FsError> {
        let inner = self.inner.lock();

        match &*inner {
            RamInodeData::File(data) => {
                if offset >= data.len() {
                    return Ok(0);
                }

                let bytes_to_copy = core::cmp::min(buf.len(), data.len() - offset);
                buf[..bytes_to_copy].copy_from_slice(&data[offset..offset + bytes_to_copy]);
                Ok(bytes_to_copy)
            }
            RamInodeData::Directory(_) => Err(FsError::IsADirectory),
        }
    }

    fn write(&self, offset: usize, buf: &[u8]) -> Result<usize, FsError> {
        let mut inner = self.inner.lock();

        match &mut *inner {
            RamInodeData::File(data) => {
                if offset > data.len() {
                    data.resize(offset, 0);
                }

                let end = offset.checked_add(buf.len()).ok_or(FsError::IoError)?;
                if end > data.len() {
                    data.resize(end, 0);
                }

                data[offset..end].copy_from_slice(buf);
                Ok(buf.len())
            }
            RamInodeData::Directory(_) => Err(FsError::IsADirectory),
        }
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>, FsError> {
        let inner = self.inner.lock();

        match &*inner {
            RamInodeData::Directory(entries) => entries
                .iter()
                .find(|(entry_name, _)| entry_name == name)
                .map(|(_, inode)| {
                    let inode: Arc<dyn Inode> = inode.clone();
                    inode
                })
                .ok_or(FsError::NotFound),
            RamInodeData::File(_) => Err(FsError::NotADirectory),
        }
    }

    fn create(&self, name: &str, file_type: FileType) -> Result<Arc<dyn Inode>, FsError> {
        if name.is_empty() {
            return Err(FsError::NotFound);
        }

        let mut inner = self.inner.lock();

        match &mut *inner {
            RamInodeData::Directory(entries) => {
                if entries.iter().any(|(entry_name, _)| entry_name == name) {
                    return Err(FsError::AlreadyExists);
                }

                let inode = Arc::new(RamInode::new(file_type, self.fs_id));
                entries.push((String::from(name), inode.clone()));
                Ok(inode)
            }
            RamInodeData::File(_) => Err(FsError::NotADirectory),
        }
    }

    fn file_type(&self) -> FileType {
        self.file_type
    }

    fn size(&self) -> usize {
        let inner = self.inner.lock();

        match &*inner {
            RamInodeData::File(data) => data.len(),
            RamInodeData::Directory(entries) => entries.len(),
        }
    }

    fn filesystem_name(&self) -> &'static str {
        "ramfs"
    }

    fn fs_id(&self) -> u64 {
        self.fs_id
    }

    fn readdir(
        &self,
        cursor: &mut DirCursor,
        _name_buf: &mut [u8],
        visit: &mut dyn for<'a> FnMut(DirEntry<'a>) -> bool,
    ) -> Result<usize, FsError> {
        let inner = self.inner.lock();
        match &*inner {
            RamInodeData::Directory(entries) => {
                let mut emitted = 0usize;
                let mut index = cursor.offset as usize;
                while index < entries.len() {
                    let (name, inode) = &entries[index];
                    let entry = DirEntry::new(name.as_str(), inode.file_type(), inode.ino());
                    emitted += 1;
                    index += 1;
                    if !visit(entry) {
                        break;
                    }
                }
                cursor.offset = index as u64;
                Ok(emitted)
            }
            RamInodeData::File(_) => Err(FsError::NotADirectory),
        }
    }
}

fn next_inode_number() -> u64 {
    static NEXT_INO: AtomicU64 = AtomicU64::new(1);
    NEXT_INO.fetch_add(1, Ordering::Relaxed)
}
