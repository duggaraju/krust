extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::Once;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Regular,
    Directory,
    CharDevice,
    BlockDevice,
    Symlink,
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
    IoError,
    NotSupported,
}

/// Directory entry returned by readdir.
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub file_type: FileType,
}

pub trait FileSystem: Send + Sync {
    fn name(&self) -> &str;
    fn root_inode(&self) -> Arc<dyn Inode>;
}

pub trait Inode: Send + Sync {
    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, FsError>;
    fn write(&self, offset: usize, buf: &[u8]) -> Result<usize, FsError>;
    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>, FsError>;
    fn create(&self, name: &str, file_type: FileType) -> Result<Arc<dyn Inode>, FsError>;
    fn file_type(&self) -> FileType;
    fn size(&self) -> usize;

    /// List directory entries. Returns Err(NotADirectory) for non-directory inodes.
    fn readdir(&self) -> Result<Vec<DirEntry>, FsError> {
        Err(FsError::NotADirectory)
    }
}

pub trait File: Send + Sync {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, FsError>;
    fn write(&mut self, buf: &[u8]) -> Result<usize, FsError>;
    fn seek(&mut self, pos: SeekFrom) -> Result<usize, FsError>;
}

static ROOT_FS: Once<Arc<dyn FileSystem>> = Once::new();

pub fn mount_root(fs: Arc<dyn FileSystem>) -> Result<(), FsError> {
    if ROOT_FS.is_completed() {
        return Err(FsError::AlreadyExists);
    }

    ROOT_FS.call_once(|| fs);
    Ok(())
}

pub fn root_fs() -> Result<&'static Arc<dyn FileSystem>, FsError> {
    ROOT_FS.get().ok_or(FsError::NotFound)
}

pub fn root_inode() -> Result<Arc<dyn Inode>, FsError> {
    Ok(root_fs()?.root_inode())
}

pub struct OpenFile {
    inode: Arc<dyn Inode>,
    cursor: usize,
}

impl OpenFile {
    pub fn new(inode: Arc<dyn Inode>) -> Self {
        Self { inode, cursor: 0 }
    }

    pub fn inode(&self) -> &Arc<dyn Inode> {
        &self.inode
    }

    pub fn position(&self) -> usize {
        self.cursor
    }
}

impl File for OpenFile {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, FsError> {
        let bytes_read = self.inode.read(self.cursor, buf)?;
        self.cursor = self.cursor.saturating_add(bytes_read);
        Ok(bytes_read)
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize, FsError> {
        let bytes_written = self.inode.write(self.cursor, buf)?;
        self.cursor = self.cursor.saturating_add(bytes_written);
        Ok(bytes_written)
    }

    fn seek(&mut self, pos: SeekFrom) -> Result<usize, FsError> {
        let size = self.inode.size();
        self.cursor = match pos {
            SeekFrom::Start(offset) => offset,
            SeekFrom::Current(delta) => apply_signed_offset(self.cursor, delta)?,
            SeekFrom::End(delta) => apply_signed_offset(size, delta)?,
        };

        Ok(self.cursor)
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
