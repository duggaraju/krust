extern crate alloc;

use alloc::format;
use alloc::sync::Arc;
use crate::fs::vfs::{self, DirCursor, DirEntry, FileType, FsError, Inode};
use crate::process;
use alloc::string::String;
use alloc::vec::Vec;

pub const STAT_TYPE_REGULAR: u32 = 1;
pub const STAT_TYPE_DIRECTORY: u32 = 2;
pub const STAT_TYPE_CHAR_DEVICE: u32 = 3;
pub const STAT_TYPE_BLOCK_DEVICE: u32 = 4;
pub const STAT_TYPE_SYMLINK: u32 = 5;

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Stat {
    pub ino: u64,
    pub file_type: u32,
    pub size: u64,
}

pub fn open(path: &str) -> Result<usize, isize> {
    let absolute = resolve_absolute_from_current_cwd(path).map_err(errno_from_fs)?;
    let file = vfs::open_path(absolute.as_str()).map_err(errno_from_fs)?;
    process::open_file(file)
}

pub fn close(fd: usize) -> Result<(), isize> {
    process::close_fd(fd)
}

pub fn read(fd: usize, buf: &mut [u8]) -> Result<usize, isize> {
    process::read_fd(fd, buf)
}

pub fn write(fd: usize, buf: &[u8]) -> Result<usize, isize> {
    process::write_fd(fd, buf)
}

pub fn readdir(
    fd: usize,
    cursor: &mut DirCursor,
    name_buf: &mut [u8],
    visit: &mut dyn for<'a> FnMut(DirEntry<'a>) -> bool,
) -> Result<usize, isize> {
    let fd_desc = process::fd_descriptor(fd)?;
    let file = vfs::open_by_descriptor(fd_desc).map_err(errno_from_fs)?;
    let ino = file.ino();
    let file_type = file.file_type();
    let mut emitted = 0usize;
    let mut inner_cursor = DirCursor {
        offset: cursor.offset.saturating_sub(2),
    };

    if cursor.offset < 2 {
        let dot_name = if cursor.offset == 0 { "." } else { ".." };
        let name = vfs::copy_name_into(dot_name, name_buf).map_err(errno_from_fs)?;
        emitted += 1;
        cursor.offset += 1;
        if !visit(DirEntry::new(name, file_type, ino)) {
            return Ok(emitted);
        }
    }

    if cursor.offset < 2 {
        let name = vfs::copy_name_into("..", name_buf).map_err(errno_from_fs)?;
        emitted += 1;
        cursor.offset += 1;
        if !visit(DirEntry::new(name, file_type, ino)) {
            return Ok(emitted);
        }
    }

    let inner_emitted = file
        .readdir(&mut inner_cursor, name_buf, visit)
        .map_err(errno_from_fs)?;
    cursor.offset = inner_cursor.offset + 2;
    Ok(emitted + inner_emitted)
}

pub fn stat(path: &str) -> Result<Stat, isize> {
    let inode = resolve_path_from_current_cwd(path).map_err(errno_from_fs)?;
    Ok(Stat {
        ino: inode.ino(),
        file_type: file_type_to_u32(inode.file_type()),
        size: inode.size() as u64,
    })
}

pub fn chdir(path: &str) -> Result<(), isize> {
    let inode = resolve_path_from_current_cwd(path).map_err(errno_from_fs)?;
    if inode.file_type() != FileType::Directory {
        return Err(-20);
    }
    let cwd_path = resolve_absolute_from_current_cwd(path).map_err(errno_from_fs)?;
    process::set_current_cwd(inode, cwd_path)?;
    Ok(())
}

pub fn mkdir(path: &str) -> Result<(), isize> {
    create_from_cwd_from_current_cwd(path, FileType::Directory)
        .map(|_| ())
        .map_err(errno_from_fs)
}

pub fn write_file(path: &str, data: &[u8]) -> Result<usize, isize> {
    let absolute = resolve_absolute_from_current_cwd(path).map_err(errno_from_fs)?;
    let inode = match vfs::lookup_path(absolute.as_str()) {
        Ok(inode) => inode,
        Err(FsError::NotFound) => {
            vfs::create_path(absolute.as_str(), FileType::Regular).map_err(errno_from_fs)?
        }
        Err(err) => return Err(errno_from_fs(err)),
    };

    inode.write(0, data).map_err(errno_from_fs)
}

pub fn read_file(path: &str, max_bytes: usize) -> Result<Vec<u8>, isize> {
    let fd = open(path)?;
    let result = (|| {
        let mut out = Vec::new();
        let mut buf = [0u8; 256];
        while out.len() < max_bytes {
            let remaining = max_bytes - out.len();
            let chunk_len = core::cmp::min(remaining, buf.len());
            let n = read(fd, &mut buf[..chunk_len])?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        Ok(out)
    })();
    let _ = close(fd);
    result
}

pub fn read_file_string(path: &str, max_bytes: usize) -> Result<String, isize> {
    let bytes = read_file(path, max_bytes)?;
    String::from_utf8(bytes).map_err(|_| -84)
}

pub fn getpid() -> isize {
    1
}

pub fn times() -> u64 {
    crate::time::uptime_ticks()
}

pub fn errno_from_fs(err: FsError) -> isize {
    match err {
        FsError::NotFound => -2,
        FsError::PermissionDenied => -13,
        FsError::NotADirectory => -20,
        FsError::IsADirectory => -21,
        FsError::AlreadyExists => -17,
        FsError::IoError => -5,
        FsError::Busy => -16,
        FsError::NotSupported => -95,
    }
}

fn file_type_to_u32(file_type: FileType) -> u32 {
    match file_type {
        FileType::Regular => STAT_TYPE_REGULAR,
        FileType::Directory => STAT_TYPE_DIRECTORY,
        FileType::CharDevice => STAT_TYPE_CHAR_DEVICE,
        FileType::BlockDevice => STAT_TYPE_BLOCK_DEVICE,
        FileType::Symlink => STAT_TYPE_SYMLINK,
    }
}

fn resolve_path_from_current_cwd(path: &str) -> Result<Arc<dyn Inode>, FsError> {
    let absolute = resolve_absolute_from_current_cwd(path)?;
    vfs::lookup_path(absolute.as_str())
}

fn resolve_absolute_from_current_cwd(path: &str) -> Result<String, FsError> {
    let path = path.trim();
    if path.is_empty() {
        return Err(FsError::NotFound);
    }
    if path.starts_with('/') {
        return vfs::normalize_absolute_path(path);
    }
    let cwd_path = process::current_cwd_path().unwrap_or_else(|| String::from("/"));
    let joined = if cwd_path == "/" {
        format!("/{}", path)
    } else {
        format!("{}/{}", cwd_path.trim_end_matches('/'), path)
    };
    vfs::normalize_absolute_path(joined.as_str())
}

fn resolve_parent_from_current_cwd(
    path: &str,
) -> Result<(alloc::sync::Arc<dyn vfs::Inode>, String), FsError> {
    let absolute = resolve_absolute_from_current_cwd(path)?;
    if absolute == "/" {
        return Err(FsError::NotFound);
    }
    let (parent_path, name) = absolute.rsplit_once('/').unwrap_or(("/", absolute.as_str()));
    let parent = if parent_path.is_empty() || parent_path == "/" {
        vfs::lookup_path("/")?
    } else {
        vfs::lookup_path(parent_path)?
    };
    Ok((parent, String::from(name)))
}

fn create_from_cwd_from_current_cwd(
    path: &str,
    file_type: FileType,
) -> Result<Arc<dyn Inode>, FsError> {
    let (parent, name) = resolve_parent_from_current_cwd(path)?;
    parent.create(name.as_str(), file_type)
}
