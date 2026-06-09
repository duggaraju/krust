extern crate alloc;

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cmp::min;
use core::str;

use super::vfs::{DirEntry, FileSystem, FileType, FsError, Inode};

const CPIO_NEWC_MAGIC: &[u8; 6] = b"070701";
const CPIO_HEADER_LEN: usize = 110;
const S_IFMT: u32 = 0o170000;
const S_IFREG: u32 = 0o100000;
const S_IFDIR: u32 = 0o040000;
const S_IFCHR: u32 = 0o020000;
const S_IFBLK: u32 = 0o060000;
const S_IFLNK: u32 = 0o120000;

pub struct InitrdFs {
    root: Arc<InitrdInode>,
}

impl InitrdFs {
    pub fn new(data: &'static [u8]) -> Result<Self, FsError> {
        let mut builder = BuilderNode::directory();
        let mut offset = 0;
        let mut found_trailer = false;

        while offset < data.len() {
            let entry = ParsedEntry::parse(data, offset)?;
            offset = entry.next_offset;

            if entry.name == "TRAILER!!!" {
                found_trailer = true;
                break;
            }

            let path = normalize_path(entry.name);
            if path.is_empty() {
                if entry.file_type != FileType::Directory {
                    return Err(FsError::IoError);
                }
                continue;
            }

            builder.insert(&path, entry.file_type, entry.contents)?;
        }

        if !found_trailer {
            return Err(FsError::IoError);
        }

        Ok(Self {
            root: builder.into_inode(),
        })
    }
}

impl FileSystem for InitrdFs {
    fn name(&self) -> &str {
        "initrd"
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }
}

pub struct InitrdInode {
    file_type: FileType,
    inner: InitrdInodeData,
}

enum InitrdInodeData {
    File(&'static [u8]),
    Directory(Vec<(String, Arc<InitrdInode>)>),
}

impl Inode for InitrdInode {
    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, FsError> {
        match &self.inner {
            InitrdInodeData::File(data) => {
                if offset >= data.len() {
                    return Ok(0);
                }

                let bytes_to_copy = min(buf.len(), data.len() - offset);
                buf[..bytes_to_copy].copy_from_slice(&data[offset..offset + bytes_to_copy]);
                Ok(bytes_to_copy)
            }
            InitrdInodeData::Directory(_) => Err(FsError::IsADirectory),
        }
    }

    fn write(&self, _offset: usize, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::NotSupported)
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>, FsError> {
        match &self.inner {
            InitrdInodeData::Directory(children) => children
                .iter()
                .find(|(child_name, _)| child_name == name)
                .map(|(_, inode)| {
                    let inode: Arc<dyn Inode> = inode.clone();
                    inode
                })
                .ok_or(FsError::NotFound),
            InitrdInodeData::File(_) => Err(FsError::NotADirectory),
        }
    }

    fn create(&self, _name: &str, _file_type: FileType) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotSupported)
    }

    fn file_type(&self) -> FileType {
        self.file_type
    }

    fn size(&self) -> usize {
        match &self.inner {
            InitrdInodeData::File(data) => data.len(),
            InitrdInodeData::Directory(children) => children.len(),
        }
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, FsError> {
        match &self.inner {
            InitrdInodeData::Directory(children) => Ok(children
                .iter()
                .map(|(name, inode)| DirEntry {
                    name: name.clone(),
                    file_type: inode.file_type(),
                })
                .collect()),
            InitrdInodeData::File(_) => Err(FsError::NotADirectory),
        }
    }
}

pub fn parse(data: &'static [u8]) -> Result<InitrdFs, FsError> {
    InitrdFs::new(data)
}

struct ParsedEntry<'a> {
    name: &'a str,
    file_type: FileType,
    contents: &'static [u8],
    next_offset: usize,
}

impl<'a> ParsedEntry<'a> {
    fn parse(data: &'static [u8], offset: usize) -> Result<Self, FsError> {
        let header_end = offset
            .checked_add(CPIO_HEADER_LEN)
            .ok_or(FsError::IoError)?;
        let header = data.get(offset..header_end).ok_or(FsError::IoError)?;

        if &header[..6] != CPIO_NEWC_MAGIC {
            return Err(FsError::IoError);
        }

        let mode = parse_hex_u32(&header[14..22])?;
        let filesize = parse_hex_usize(&header[54..62])?;
        let namesize = parse_hex_usize(&header[94..102])?;

        if namesize == 0 {
            return Err(FsError::IoError);
        }

        let name_start = header_end;
        let name_end = name_start.checked_add(namesize).ok_or(FsError::IoError)?;
        let raw_name = data.get(name_start..name_end).ok_or(FsError::IoError)?;
        let (nul, name_bytes) = raw_name.split_last().ok_or(FsError::IoError)?;
        if *nul != 0 {
            return Err(FsError::IoError);
        }

        let name = str::from_utf8(name_bytes).map_err(|_| FsError::IoError)?;
        let file_start = align_up(name_end, 4)?;
        let file_end = file_start.checked_add(filesize).ok_or(FsError::IoError)?;
        let contents = data.get(file_start..file_end).ok_or(FsError::IoError)?;
        let next_offset = align_up(file_end, 4)?;

        Ok(Self {
            name,
            file_type: mode_to_file_type(mode)?,
            contents,
            next_offset,
        })
    }
}

fn normalize_path(path: &str) -> String {
    let trimmed = path.trim_matches('/');
    if trimmed == "." {
        return String::new();
    }

    trimmed
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect::<Vec<_>>()
        .join("/")
}

fn parse_hex_u32(bytes: &[u8]) -> Result<u32, FsError> {
    let text = str::from_utf8(bytes).map_err(|_| FsError::IoError)?;
    u32::from_str_radix(text, 16).map_err(|_| FsError::IoError)
}

fn parse_hex_usize(bytes: &[u8]) -> Result<usize, FsError> {
    let value = parse_hex_u32(bytes)?;
    usize::try_from(value).map_err(|_| FsError::IoError)
}

fn align_up(value: usize, align: usize) -> Result<usize, FsError> {
    let mask = align.checked_sub(1).ok_or(FsError::IoError)?;
    value
        .checked_add(mask)
        .map(|aligned| aligned & !mask)
        .ok_or(FsError::IoError)
}

fn mode_to_file_type(mode: u32) -> Result<FileType, FsError> {
    match mode & S_IFMT {
        S_IFREG => Ok(FileType::Regular),
        S_IFDIR => Ok(FileType::Directory),
        S_IFCHR => Ok(FileType::CharDevice),
        S_IFBLK => Ok(FileType::BlockDevice),
        S_IFLNK => Ok(FileType::Symlink),
        _ => Err(FsError::NotSupported),
    }
}

struct BuilderNode {
    file_type: FileType,
    inner: BuilderNodeData,
}

enum BuilderNodeData {
    File(&'static [u8]),
    Directory(Vec<(String, BuilderNode)>),
}

impl BuilderNode {
    fn directory() -> Self {
        Self {
            file_type: FileType::Directory,
            inner: BuilderNodeData::Directory(Vec::new()),
        }
    }

    fn file(file_type: FileType, contents: &'static [u8]) -> Self {
        Self {
            file_type,
            inner: BuilderNodeData::File(contents),
        }
    }

    fn insert(
        &mut self,
        path: &str,
        file_type: FileType,
        contents: &'static [u8],
    ) -> Result<(), FsError> {
        let mut current = self;
        let mut segments = path.split('/').peekable();

        while let Some(segment) = segments.next() {
            if segments.peek().is_none() {
                return current.insert_child(segment, file_type, contents);
            }

            current = current.get_or_create_dir(segment)?;
        }

        Ok(())
    }

    fn get_or_create_dir(&mut self, name: &str) -> Result<&mut BuilderNode, FsError> {
        match &mut self.inner {
            BuilderNodeData::Directory(children) => {
                if let Some(index) = children
                    .iter()
                    .position(|(child_name, _)| child_name == name)
                {
                    let (_, child) = &mut children[index];
                    if child.file_type != FileType::Directory {
                        return Err(FsError::NotADirectory);
                    }
                    return Ok(child);
                }

                children.push((name.to_string(), BuilderNode::directory()));
                let (_, child) = children.last_mut().ok_or(FsError::IoError)?;
                Ok(child)
            }
            BuilderNodeData::File(_) => Err(FsError::NotADirectory),
        }
    }

    fn insert_child(
        &mut self,
        name: &str,
        file_type: FileType,
        contents: &'static [u8],
    ) -> Result<(), FsError> {
        match &mut self.inner {
            BuilderNodeData::Directory(children) => {
                if let Some((_, child)) = children
                    .iter_mut()
                    .find(|(child_name, _)| child_name == name)
                {
                    if child.file_type == FileType::Directory && file_type == FileType::Directory {
                        return Ok(());
                    }
                    return Err(FsError::AlreadyExists);
                }

                let node = if file_type == FileType::Directory {
                    BuilderNode::directory()
                } else {
                    BuilderNode::file(file_type, contents)
                };
                children.push((name.to_string(), node));
                Ok(())
            }
            BuilderNodeData::File(_) => Err(FsError::NotADirectory),
        }
    }

    fn into_inode(self) -> Arc<InitrdInode> {
        let inner = match self.inner {
            BuilderNodeData::File(contents) => InitrdInodeData::File(contents),
            BuilderNodeData::Directory(children) => InitrdInodeData::Directory(
                children
                    .into_iter()
                    .map(|(name, child)| (name, child.into_inode()))
                    .collect(),
            ),
        };

        Arc::new(InitrdInode {
            file_type: self.file_type,
            inner,
        })
    }
}
