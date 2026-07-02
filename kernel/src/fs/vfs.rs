extern crate alloc;

use alloc::borrow::ToOwned;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::str;
use core::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "drivers")]
use crate::drivers::registry;
#[cfg(feature = "drivers")]
use crate::drivers::traits::{DeviceError, DeviceType};
use spin::{Mutex, Once};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Regular,
    Directory,
    CharDevice,
    BlockDevice,
    Symlink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOpenMode {
    ReadOnly,
    WriteOnly,
    ReadWrite,
}

impl FileOpenMode {
    pub fn as_str(self) -> &'static str {
        match self {
            FileOpenMode::ReadOnly => "ro",
            FileOpenMode::WriteOnly => "wo",
            FileOpenMode::ReadWrite => "rw",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FileDescriptor {
    pub fs_id: u64,
    pub major: u16,
    pub minor: u16,
    pub inode: u64,
    pub mode: FileOpenMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekFrom {
    Start(usize),
    Current(isize),
    End(isize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsError {
    NotFound,
    PermissionDenied,
    NotADirectory,
    IsADirectory,
    AlreadyExists,
    Busy,
    IoError,
    NotSupported,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DirCursor {
    pub offset: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct DirEntry<'a> {
    pub name: &'a str,
    pub file_type: FileType,
    pub ino: u64,
    pub uid: u32,
    pub gid: u32,
    pub mode: u16,
    pub accessed_at: u64,
    pub modified_at: u64,
    pub created_at: u64,
}

impl<'a> DirEntry<'a> {
    pub fn new(name: &'a str, file_type: FileType, ino: u64) -> Self {
        Self {
            name,
            file_type,
            ino,
            uid: 0,
            gid: 0,
            mode: 0,
            accessed_at: 0,
            modified_at: 0,
            created_at: 0,
        }
    }
}

pub trait FileSystem: Send + Sync {
    fn name(&self) -> &str;
    fn root_inode(&self) -> Arc<dyn Inode>;
    fn fs_id(&self) -> u64 {
        0
    }
    fn ioctl(&self, _request: usize, _arg: usize) -> Result<usize, FsError> {
        Err(FsError::NotSupported)
    }

    fn readdir(
        &self,
        inode: &dyn Inode,
        cursor: &mut DirCursor,
        name_buf: &mut [u8],
        visit: &mut dyn for<'a> FnMut(DirEntry<'a>) -> bool,
    ) -> Result<usize, FsError> {
        inode.readdir(cursor, name_buf, visit)
    }
}

pub trait Inode: Send + Sync {
    fn ino(&self) -> u64 {
        0
    }

    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, FsError>;
    fn write(&self, offset: usize, buf: &[u8]) -> Result<usize, FsError>;
    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>, FsError>;
    fn create(&self, name: &str, file_type: FileType) -> Result<Arc<dyn Inode>, FsError>;
    fn file_type(&self) -> FileType;
    fn size(&self) -> usize;
    fn uid(&self) -> u32 {
        0
    }
    fn gid(&self) -> u32 {
        0
    }
    fn mode(&self) -> u16 {
        match self.file_type() {
            FileType::Directory => 0o755,
            FileType::CharDevice | FileType::BlockDevice => 0o660,
            FileType::Symlink => 0o777,
            FileType::Regular => 0o644,
        }
    }
    fn fs_id(&self) -> u64 {
        0
    }
    fn filesystem_name(&self) -> &'static str {
        "unknown"
    }
    fn ioctl(&self, _request: usize, _arg: usize) -> Result<usize, FsError> {
        Err(FsError::NotSupported)
    }
    fn device_numbers(&self) -> Option<(u16, u16)> {
        None
    }

    /// List directory entries in batches. Returns Err(NotADirectory) for non-directory inodes.
    fn readdir(
        &self,
        cursor: &mut DirCursor,
        name_buf: &mut [u8],
        visit: &mut dyn for<'a> FnMut(DirEntry<'a>) -> bool,
    ) -> Result<usize, FsError> {
        let _ = (cursor, name_buf, visit);
        Err(FsError::NotADirectory)
    }
}

pub trait File: Send + Sync {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, FsError>;
    fn write(&mut self, buf: &[u8]) -> Result<usize, FsError>;
    fn seek(&mut self, pos: SeekFrom) -> Result<usize, FsError>;
    fn ioctl(&mut self, request: usize, arg: usize) -> Result<usize, FsError>;
}

static FILESYSTEM_MANAGER: Once<Mutex<FileSystemManager>> = Once::new();
pub const DEFAULT_SYMLINK_RESOLUTION_LIMIT: usize = 10;
type InodeTuple = (u64, u16, u16, u64);
static INODE_TUPLE_INDEX: Once<Mutex<BTreeMap<InodeTuple, Arc<dyn Inode>>>> = Once::new();
static INODE_REF_COUNTS: Once<Mutex<BTreeMap<InodeTuple, usize>>> = Once::new();
static NEXT_FILESYSTEM_ID: AtomicU64 = AtomicU64::new(1);

fn filesystem_manager() -> &'static Mutex<FileSystemManager> {
    FILESYSTEM_MANAGER.call_once(|| Mutex::new(FileSystemManager::new()))
}

fn inode_tuple_index() -> &'static Mutex<BTreeMap<InodeTuple, Arc<dyn Inode>>> {
    INODE_TUPLE_INDEX.call_once(|| Mutex::new(BTreeMap::new()))
}

fn inode_ref_counts() -> &'static Mutex<BTreeMap<InodeTuple, usize>> {
    INODE_REF_COUNTS.call_once(|| Mutex::new(BTreeMap::new()))
}

#[derive(Clone)]
struct MountPoint {
    path: String,
    fs: Arc<dyn FileSystem>,
}

pub fn mount_root(fs: Arc<dyn FileSystem>) -> Result<(), FsError> {
    filesystem_manager().lock().mount_root(fs)
}

pub fn root_fs() -> Result<Arc<dyn FileSystem>, FsError> {
    filesystem_manager().lock().root_fs()
}

pub fn filesystem_by_id(fs_id: u64) -> Result<Arc<dyn FileSystem>, FsError> {
    filesystem_manager().lock().filesystem_by_id(fs_id)
}

pub fn root_inode() -> Result<Arc<dyn Inode>, FsError> {
    filesystem_manager().lock().root_inode()
}

pub fn mount(path: &str, fs: Arc<dyn FileSystem>) -> Result<(), FsError> {
    let normalized = normalize_absolute_path(path)?;
    filesystem_manager().lock().mount(&normalized, fs)
}

pub fn unmount(path: &str) -> Result<(), FsError> {
    let normalized = normalize_absolute_path(path)?;
    filesystem_manager().lock().unmount(&normalized)
}

pub fn lookup_path(path: &str) -> Result<Arc<dyn Inode>, FsError> {
    lookup_path_with_options(path, false)
}

/// Resolve all symlinks in `path` and return the canonical absolute path string.
///
/// Unlike `lookup_path_with_options`, this returns the *path* rather than the inode,
/// which lets callers (e.g. `execve`) use the resolved path for loading while
/// keeping the original path for user-visible purposes (argv, AT_EXECFN).
pub fn resolve_symlink_path(path: &str) -> Result<String, FsError> {
    let normalized = normalize_absolute_path(path)?;
    let mut followed_symlinks = 0usize;
    let max_follows = DEFAULT_SYMLINK_RESOLUTION_LIMIT;
    let mut current_path = normalized;

    loop {
        let segments: Vec<&str> = split_segments(current_path.as_str()).collect();
        let mut restarted = false;

        for index in 0..segments.len() {
            let prefix = join_absolute_segments(&segments[..=index]);
            let inode = lookup_path_without_symlink_resolution(prefix.as_str())?;
            if inode.file_type() != FileType::Symlink {
                continue;
            }

            if followed_symlinks >= max_follows {
                return Err(FsError::NotSupported);
            }
            followed_symlinks = followed_symlinks.saturating_add(1);

            let target = read_symlink_target(inode.as_ref())?;
            let parent = if index == 0 {
                String::from("/")
            } else {
                join_absolute_segments(&segments[..index])
            };
            let suffix = if index + 1 < segments.len() {
                join_relative_segments(&segments[index + 1..])
            } else {
                String::new()
            };

            let mut expanded = if target.starts_with('/') {
                target
            } else if parent == "/" {
                let mut combined = String::from("/");
                combined.push_str(target.as_str());
                combined
            } else {
                let mut combined = parent;
                if !combined.ends_with('/') {
                    combined.push('/');
                }
                combined.push_str(target.as_str());
                combined
            };

            if !suffix.is_empty() {
                if !expanded.ends_with('/') {
                    expanded.push('/');
                }
                expanded.push_str(suffix.as_str());
            }

            current_path = normalize_absolute_path(expanded.as_str())?;
            restarted = true;
            break;
        }

        if !restarted {
            return Ok(current_path);
        }
    }
}

pub fn lookup_path_with_options(
    path: &str,
    resolve_symlinks: bool,
) -> Result<Arc<dyn Inode>, FsError> {
    let normalized = normalize_absolute_path(path)?;
    if !resolve_symlinks {
        return lookup_path_without_symlink_resolution(normalized.as_str());
    }

    let mut followed_symlinks = 0usize;
    let max_follows = DEFAULT_SYMLINK_RESOLUTION_LIMIT;
    let mut current_path = normalized;

    loop {
        let segments: Vec<&str> = split_segments(current_path.as_str()).collect();
        let mut restarted = false;

        for index in 0..segments.len() {
            let prefix = join_absolute_segments(&segments[..=index]);
            let inode = lookup_path_without_symlink_resolution(prefix.as_str())?;
            if inode.file_type() != FileType::Symlink {
                continue;
            }

            if followed_symlinks >= max_follows {
                return Err(FsError::NotSupported);
            }
            followed_symlinks = followed_symlinks.saturating_add(1);

            let target = read_symlink_target(inode.as_ref())?;
            let parent = if index == 0 {
                String::from("/")
            } else {
                join_absolute_segments(&segments[..index])
            };
            let suffix = if index + 1 < segments.len() {
                join_relative_segments(&segments[index + 1..])
            } else {
                String::new()
            };

            let mut expanded = if target.starts_with('/') {
                target
            } else if parent == "/" {
                let mut combined = String::from("/");
                combined.push_str(target.as_str());
                combined
            } else {
                let mut combined = parent;
                if !combined.ends_with('/') {
                    combined.push('/');
                }
                combined.push_str(target.as_str());
                combined
            };

            if !suffix.is_empty() {
                if !expanded.ends_with('/') {
                    expanded.push('/');
                }
                expanded.push_str(suffix.as_str());
            }

            current_path = normalize_absolute_path(expanded.as_str())?;
            restarted = true;
            break;
        }

        if !restarted {
            return lookup_path_without_symlink_resolution(current_path.as_str());
        }
    }
}

fn lookup_path_without_symlink_resolution(path: &str) -> Result<Arc<dyn Inode>, FsError> {
    let mut resolved = filesystem_manager().lock().resolve_path(path)?;

    for segment in split_segments(resolved.rest.as_str()) {
        resolved.inode = resolved.inode.lookup(segment)?;
    }

    Ok(resolved.inode)
}

pub fn lookup_path_from_inode(
    start: Arc<dyn Inode>,
    path: &str,
) -> Result<Arc<dyn Inode>, FsError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(FsError::NotFound);
    }
    if trimmed.starts_with('/') {
        return lookup_path(trimmed);
    }

    let mut inode = start;
    for segment in split_segments(trimmed) {
        if segment == "." {
            continue;
        }
        if segment == ".." {
            return Err(FsError::NotSupported);
        }
        inode = inode.lookup(segment)?;
    }

    Ok(inode)
}

pub fn open_path(path: &str) -> Result<OpenFile, FsError> {
    let normalized = normalize_absolute_path(path)?;
    let mut resolved = filesystem_manager().lock().resolve_path(&normalized)?;
    for segment in split_segments(resolved.rest.as_str()) {
        resolved.inode = resolved.inode.lookup(segment)?;
    }
    OpenFile::new(resolved.inode, Some(resolved.mount_path))
}

pub fn create_path(path: &str, file_type: FileType) -> Result<Arc<dyn Inode>, FsError> {
    let normalized = normalize_absolute_path(path)?;
    if normalized == "/" {
        return Err(FsError::AlreadyExists);
    }

    let (parent_path, name) = split_parent_name(&normalized)?;
    let parent = lookup_path(parent_path)?;
    parent.create(name, file_type)
}

pub fn copy_name_into<'a>(name: &str, name_buf: &'a mut [u8]) -> Result<&'a str, FsError> {
    if name.len() > name_buf.len() {
        return Err(FsError::IoError);
    }

    name_buf[..name.len()].copy_from_slice(name.as_bytes());
    str::from_utf8(&name_buf[..name.len()]).map_err(|_| FsError::IoError)
}

pub fn write_u64_decimal_into<'a>(value: u64, name_buf: &'a mut [u8]) -> Result<&'a str, FsError> {
    let mut tmp = [0u8; 20];
    let mut len = 0usize;
    let mut n = value;
    loop {
        tmp[len] = b'0' + (n % 10) as u8;
        len += 1;
        n /= 10;
        if n == 0 {
            break;
        }
    }

    if len > name_buf.len() {
        return Err(FsError::IoError);
    }

    for i in 0..len {
        name_buf[i] = tmp[len - 1 - i];
    }
    str::from_utf8(&name_buf[..len]).map_err(|_| FsError::IoError)
}

pub fn lookup_inode_by_tuple(
    fs_id: u64,
    major: u16,
    minor: u16,
    ino: u64,
) -> Option<Arc<dyn Inode>> {
    inode_tuple_index()
        .lock()
        .get(&(fs_id, major, minor, ino))
        .cloned()
}

pub fn open_by_descriptor(descriptor: FileDescriptor) -> Result<OpenFile, FsError> {
    let inode = lookup_inode_by_tuple(
        descriptor.fs_id,
        descriptor.major,
        descriptor.minor,
        descriptor.inode,
    )
    .ok_or(FsError::NotFound)?;
    OpenFile::new(inode, None)
}

fn tuple_for_inode(inode: &dyn Inode) -> InodeTuple {
    let fs_id = inode.fs_id();
    let (major, minor) = inode.device_numbers().unwrap_or((0, 0));
    (fs_id, major, minor, inode.ino())
}

struct ResolvedInode {
    inode: Arc<dyn Inode>,
    mount_path: String,
    rest: String,
}

#[derive(Default)]
struct FileSystemManager {
    root: Option<Arc<dyn FileSystem>>,
    mounts: Vec<MountPoint>,
    open_inodes: BTreeMap<String, BTreeMap<u64, usize>>,
}

impl FileSystemManager {
    fn new() -> Self {
        Self::default()
    }

    fn mount_root(&mut self, fs: Arc<dyn FileSystem>) -> Result<(), FsError> {
        // Allow replacing the bootstrap root (e.g. ramfs) with a runtime root
        // filesystem (e.g. FAT/ext4) during late mount registration.
        self.root = Some(fs);
        Ok(())
    }

    fn root_fs(&self) -> Result<Arc<dyn FileSystem>, FsError> {
        self.root.as_ref().cloned().ok_or(FsError::NotFound)
    }

    fn filesystem_by_id(&self, fs_id: u64) -> Result<Arc<dyn FileSystem>, FsError> {
        if let Some(root) = self.root.as_ref() {
            if root.fs_id() == fs_id {
                return Ok(Arc::clone(root));
            }
        }
        for mount in &self.mounts {
            if mount.fs.fs_id() == fs_id {
                return Ok(Arc::clone(&mount.fs));
            }
        }
        Err(FsError::NotFound)
    }

    fn root_inode(&self) -> Result<Arc<dyn Inode>, FsError> {
        Ok(self.root_fs()?.root_inode())
    }

    fn mount(&mut self, normalized: &str, fs: Arc<dyn FileSystem>) -> Result<(), FsError> {
        if normalized == "/" {
            return Err(FsError::AlreadyExists);
        }

        let mountpoint = self.resolve_path(normalized)?.inode;
        if mountpoint.file_type() != FileType::Directory {
            return Err(FsError::NotADirectory);
        }

        if self.mounts.iter().any(|entry| entry.path == normalized) {
            return Err(FsError::AlreadyExists);
        }
        self.mounts.push(MountPoint {
            path: String::from(normalized),
            fs,
        });
        Ok(())
    }

    fn unmount(&mut self, normalized: &str) -> Result<(), FsError> {
        if normalized == "/" {
            return Err(FsError::NotSupported);
        }
        let Some(index) = self.mounts.iter().position(|m| m.path == normalized) else {
            return Err(FsError::NotFound);
        };
        if self
            .open_inodes
            .get(normalized)
            .is_some_and(|entries| !entries.is_empty())
        {
            return Err(FsError::NotSupported);
        }
        self.mounts.remove(index);
        self.open_inodes.remove(normalized);
        Ok(())
    }

    fn resolve_path(&self, path: &str) -> Result<ResolvedInode, FsError> {
        if path == "/" {
            return Ok(ResolvedInode {
                inode: self.root_inode()?,
                mount_path: String::from("/"),
                rest: String::new(),
            });
        }

        let selected = self
            .mounts
            .iter()
            .filter(|mount| {
                if path == mount.path {
                    return true;
                }
                if path.len() <= mount.path.len() {
                    return false;
                }
                path.starts_with(mount.path.as_str())
                    && path.as_bytes().get(mount.path.len()) == Some(&b'/')
            })
            .max_by_key(|mount| mount.path.len());

        if let Some(mount) = selected {
            let rest = if path == mount.path {
                ""
            } else {
                &path[mount.path.len() + 1..]
            };
            return Ok(ResolvedInode {
                inode: mount.fs.root_inode(),
                mount_path: mount.path.clone(),
                rest: String::from(rest),
            });
        }

        Ok(ResolvedInode {
            inode: self.root_inode()?,
            mount_path: String::from("/"),
            rest: String::from(&path[1..]),
        })
    }

    fn track_open_inode(&mut self, mount_path: &str, ino: u64) {
        let entry = self
            .open_inodes
            .entry(String::from(mount_path))
            .or_default()
            .entry(ino)
            .or_insert(0);
        *entry = entry.saturating_add(1);
    }

    fn track_close_inode(&mut self, mount_path: &str, ino: u64) {
        let Some(mount_entries) = self.open_inodes.get_mut(mount_path) else {
            return;
        };
        let Some(count) = mount_entries.get_mut(&ino) else {
            return;
        };
        if *count <= 1 {
            mount_entries.remove(&ino);
        } else {
            *count -= 1;
        }
    }
}

pub(crate) fn normalize_absolute_path(path: &str) -> Result<String, FsError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(FsError::NotFound);
    }

    let absolute = if trimmed.starts_with('/') {
        trimmed.to_owned()
    } else {
        let mut prefixed = String::from("/");
        prefixed.push_str(trimmed);
        prefixed
    };

    let mut normalized = String::from("/");
    for segment in split_segments(&absolute[1..]) {
        if segment == "." {
            continue;
        }
        if segment == ".." {
            if normalized.len() > 1 {
                if let Some((parent, _)) = normalized.rsplit_once('/') {
                    normalized.truncate(parent.len().max(1));
                } else {
                    normalized.truncate(1);
                }
            }
            continue;
        }

        if normalized.len() > 1 {
            normalized.push('/');
        }
        normalized.push_str(segment);
    }

    Ok(normalized)
}

fn split_segments(path: &str) -> impl Iterator<Item = &str> {
    path.split('/').filter(|segment| !segment.is_empty())
}

fn join_absolute_segments(segments: &[&str]) -> String {
    if segments.is_empty() {
        return String::from("/");
    }

    let mut out = String::from("/");
    for (index, segment) in segments.iter().enumerate() {
        if index > 0 {
            out.push('/');
        }
        out.push_str(segment);
    }
    out
}

fn join_relative_segments(segments: &[&str]) -> String {
    let mut out = String::new();
    for (index, segment) in segments.iter().enumerate() {
        if index > 0 {
            out.push('/');
        }
        out.push_str(segment);
    }
    out
}

fn read_symlink_target(inode: &dyn Inode) -> Result<String, FsError> {
    if inode.file_type() != FileType::Symlink {
        return Err(FsError::NotSupported);
    }

    let size = inode.size();
    let mut buf = vec![0u8; size];
    let read = inode.read(0, buf.as_mut_slice())?;
    buf.truncate(read);
    String::from_utf8(buf).map_err(|_| FsError::IoError)
}

fn split_parent_name(path: &str) -> Result<(&str, &str), FsError> {
    let Some((parent, name)) = path.rsplit_once('/') else {
        return Err(FsError::NotFound);
    };

    if name.is_empty() {
        return Err(FsError::NotFound);
    }

    let parent = if parent.is_empty() { "/" } else { parent };
    Ok((parent, name))
}

pub struct OpenFile {
    inode: Arc<dyn Inode>,
    cursor: usize,
    mount_path: Option<String>,
    filesystem: Arc<dyn FileSystem>,
    tracked_open: bool,
    tuple_key: InodeTuple,
    tracked_ref: bool,
    #[cfg(feature = "drivers")]
    device: Option<registry::DeviceDescriptor>,
}

impl OpenFile {
    pub fn new(inode: Arc<dyn Inode>, mount_path: Option<String>) -> Result<Self, FsError> {
        let key = tuple_for_inode(inode.as_ref());
        let filesystem = match inode.fs_id() {
            0 => crate::fs::filesystem(inode.filesystem_name())?,
            fs_id => filesystem_by_id(fs_id)
                .or_else(|_| crate::fs::filesystem(inode.filesystem_name()))?,
        };
        let mut file = Self {
            inode,
            cursor: 0,
            mount_path,
            filesystem,
            tracked_open: false,
            tuple_key: key,
            tracked_ref: false,
            #[cfg(feature = "drivers")]
            device: None,
        };
        if let Some(path) = file.mount_path.clone() {
            filesystem_manager()
                .lock()
                .track_open_inode(path.as_str(), file.inode.ino());
            file.tracked_open = true;
        }
        #[cfg(feature = "drivers")]
        if let Some(descriptor) = file.lookup_device() {
            let descriptor =
                registry::open(descriptor.name.as_str()).map_err(fs_error_from_registry)?;
            if let Err(err) = descriptor.device.open() {
                let _ = registry::close(descriptor.name.as_str());
                return Err(fs_error_from_device(err));
            }
            file.device = Some(descriptor);
        }
        inode_tuple_index()
            .lock()
            .insert(key, Arc::clone(&file.inode));
        {
            let mut counts = inode_ref_counts().lock();
            let count = counts.entry(key).or_insert(0);
            *count = count.saturating_add(1);
            file.tracked_ref = true;
        }
        Ok(file)
    }

    pub fn inode(&self) -> &Arc<dyn Inode> {
        &self.inode
    }

    pub fn filesystem(&self) -> Arc<dyn FileSystem> {
        Arc::clone(&self.filesystem)
    }

    pub fn position(&self) -> usize {
        self.cursor
    }

    pub fn descriptor(&self, mode: FileOpenMode) -> FileDescriptor {
        let (major, minor) = self.inode.device_numbers().unwrap_or((0, 0));
        FileDescriptor {
            fs_id: self.inode.fs_id(),
            major,
            minor,
            inode: self.inode.ino(),
            mode,
        }
    }

    pub fn ino(&self) -> u64 {
        self.inode.ino()
    }

    pub fn file_type(&self) -> FileType {
        self.inode.file_type()
    }

    pub fn readdir(
        &self,
        cursor: &mut DirCursor,
        name_buf: &mut [u8],
        visit: &mut dyn for<'a> FnMut(DirEntry<'a>) -> bool,
    ) -> Result<usize, FsError> {
        self.filesystem
            .readdir(self.inode.as_ref(), cursor, name_buf, visit)
    }

    #[cfg(feature = "drivers")]
    fn lookup_device(&self) -> Option<registry::DeviceDescriptor> {
        if let Some(descriptor) = self.device.as_ref() {
            return Some(descriptor.clone());
        }
        let device_type = match self.inode.file_type() {
            FileType::CharDevice => DeviceType::Char,
            FileType::BlockDevice => DeviceType::Block,
            _ => return None,
        };
        let (major, minor) = self.inode.device_numbers()?;
        registry::get_descriptor_by_number(device_type, major, minor)
    }

    pub fn close(&mut self) -> Result<(), FsError> {
        #[cfg(feature = "drivers")]
        if let Some(device) = self.device.take() {
            registry::close(device.name.as_str()).map_err(fs_error_from_registry)?;
            device.device.close().map_err(fs_error_from_device)?;
        }
        self.release_tracking();
        self.release_refcount();
        Ok(())
    }

    fn release_tracking(&mut self) {
        if !self.tracked_open {
            return;
        }
        if let Some(path) = self.mount_path.as_ref() {
            filesystem_manager()
                .lock()
                .track_close_inode(path.as_str(), self.inode.ino());
        }
        self.tracked_open = false;
    }

    fn release_refcount(&mut self) {
        if !self.tracked_ref {
            return;
        }
        let mut counts = inode_ref_counts().lock();
        if let Some(count) = counts.get_mut(&self.tuple_key) {
            if *count <= 1 {
                counts.remove(&self.tuple_key);
            } else {
                *count -= 1;
            }
        }
        self.tracked_ref = false;
    }
}

impl File for OpenFile {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, FsError> {
        #[cfg(feature = "drivers")]
        let bytes_read = if let Some(device) = self.lookup_device() {
            device
                .device
                .read(self.cursor, buf)
                .map_err(fs_error_from_device)?
        } else {
            self.inode.read(self.cursor, buf)?
        };
        #[cfg(not(feature = "drivers"))]
        let bytes_read = self.inode.read(self.cursor, buf)?;
        self.cursor = self.cursor.saturating_add(bytes_read);
        Ok(bytes_read)
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize, FsError> {
        #[cfg(feature = "drivers")]
        let bytes_written = if let Some(device) = self.lookup_device() {
            device
                .device
                .write(self.cursor, buf)
                .map_err(fs_error_from_device)?
        } else {
            self.inode.write(self.cursor, buf)?
        };
        #[cfg(not(feature = "drivers"))]
        let bytes_written = self.inode.write(self.cursor, buf)?;
        self.cursor = self.cursor.saturating_add(bytes_written);
        Ok(bytes_written)
    }

    fn seek(&mut self, pos: SeekFrom) -> Result<usize, FsError> {
        #[cfg(feature = "drivers")]
        {
            self.cursor = if let Some(device) = self.lookup_device() {
                device
                    .device
                    .seek(self.cursor, pos)
                    .map_err(fs_error_from_device)?
            } else {
                let size = self.inode.size();
                match pos {
                    SeekFrom::Start(offset) => offset,
                    SeekFrom::Current(delta) => apply_signed_offset(self.cursor, delta)?,
                    SeekFrom::End(delta) => apply_signed_offset(size, delta)?,
                }
            };
        }
        #[cfg(not(feature = "drivers"))]
        {
            let size = self.inode.size();
            self.cursor = match pos {
                SeekFrom::Start(offset) => offset,
                SeekFrom::Current(delta) => apply_signed_offset(self.cursor, delta)?,
                SeekFrom::End(delta) => apply_signed_offset(size, delta)?,
            };
        }

        Ok(self.cursor)
    }

    fn ioctl(&mut self, request: usize, arg: usize) -> Result<usize, FsError> {
        #[cfg(feature = "drivers")]
        if let Some(device) = self.lookup_device() {
            return device
                .device
                .ioctl(request, arg)
                .map_err(fs_error_from_device);
        }

        self.inode.ioctl(request, arg)
    }
}

impl Drop for OpenFile {
    fn drop(&mut self) {
        self.release_tracking();
        self.release_refcount();
    }
}

fn apply_signed_offset(base: usize, delta: isize) -> Result<usize, FsError> {
    if delta >= 0 {
        base.checked_add(delta as usize).ok_or(FsError::IoError)
    } else {
        base.checked_sub(delta.unsigned_abs())
            .ok_or(FsError::IoError)
    }
}

pub fn allocate_filesystem_id() -> u64 {
    NEXT_FILESYSTEM_ID.fetch_add(1, Ordering::Relaxed)
}

#[cfg(feature = "drivers")]
fn fs_error_from_device(err: DeviceError) -> FsError {
    match err {
        DeviceError::IoError | DeviceError::NotReady => FsError::IoError,
        DeviceError::InvalidArgument | DeviceError::NotFound => FsError::NotFound,
        DeviceError::Busy => FsError::Busy,
        DeviceError::NotSupported => FsError::NotSupported,
    }
}

#[cfg(feature = "drivers")]
fn fs_error_from_registry(err: registry::RegistryError) -> FsError {
    match err {
        registry::RegistryError::InvalidName | registry::RegistryError::NotFound => {
            FsError::NotFound
        }
        registry::RegistryError::NameInUse
        | registry::RegistryError::DeviceNumberInUse
        | registry::RegistryError::Busy => FsError::Busy,
    }
}
