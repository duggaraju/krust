extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::Mutex;

use super::vfs::{DirEntry, FileSystem, FileType, FsError, Inode};

pub struct RamFs {
    root: Arc<RamInode>,
}

impl RamFs {
    pub fn new() -> Self {
        Self {
            root: Arc::new(RamInode::directory()),
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
}

pub struct RamInode {
    file_type: FileType,
    inner: Mutex<RamInodeData>,
}

enum RamInodeData {
    File(Vec<u8>),
    Directory(Vec<(String, Arc<RamInode>)>),
}

impl RamInode {
    pub fn new(file_type: FileType) -> Self {
        let inner = match file_type {
            FileType::Directory => RamInodeData::Directory(Vec::new()),
            _ => RamInodeData::File(Vec::new()),
        };

        Self {
            file_type,
            inner: Mutex::new(inner),
        }
    }

    pub fn directory() -> Self {
        Self::new(FileType::Directory)
    }
}

impl Inode for RamInode {
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

                let inode = Arc::new(RamInode::new(file_type));
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

    fn readdir(&self) -> Result<Vec<DirEntry>, FsError> {
        let inner = self.inner.lock();
        match &*inner {
            RamInodeData::Directory(entries) => Ok(entries
                .iter()
                .map(|(name, inode)| DirEntry {
                    name: name.clone(),
                    file_type: inode.file_type(),
                })
                .collect()),
            RamInodeData::File(_) => Err(FsError::NotADirectory),
        }
    }
}
