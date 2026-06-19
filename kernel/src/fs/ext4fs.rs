extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use spin::Mutex;

use ext4_rs::{BLOCK_SIZE as EXT4_BLOCK_SIZE, BlockDevice as Ext4BlockDevice, Ext4, InodeFileType};

use crate::drivers::traits::BlockDevice;
use crate::module::traits::{KernelModule, KernelRegistry, ModuleError};

use super::vfs::{DirCursor, DirEntry, FileSystem, FileType, FsError, Inode, copy_name_into};

// ext4 root inode number (always 2 in the ext4 specification)
const EXT4_ROOT_INODE: u32 = 2;
// Inode flag indicating an extent tree is used (no inline symlink data)
const INODE_FLAG_EXTENTS: u32 = 0x0008_0000;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn map_dir_entry_type(de_type: u8) -> FileType {
    match de_type {
        1 => FileType::Regular,
        2 => FileType::Directory,
        3 => FileType::CharDevice,
        4 => FileType::BlockDevice,
        7 => FileType::Symlink,
        _ => FileType::Regular,
    }
}

fn map_inode_mode(kind: InodeFileType) -> FileType {
    if kind == InodeFileType::S_IFDIR {
        FileType::Directory
    } else if kind == InodeFileType::S_IFLNK {
        FileType::Symlink
    } else if kind == InodeFileType::S_IFCHR {
        FileType::CharDevice
    } else if kind == InodeFileType::S_IFBLK {
        FileType::BlockDevice
    } else {
        FileType::Regular
    }
}

// ---------------------------------------------------------------------------
// Block device adapter
// ---------------------------------------------------------------------------

/// Bridges krust's `BlockDevice` to ext4_rs's `BlockDevice`.
/// ext4_rs reads/writes one 4 KiB block per call (byte-addressed offset).
struct BlockDeviceAdapter {
    inner: Arc<dyn BlockDevice>,
}

impl BlockDeviceAdapter {
    fn new(inner: Arc<dyn BlockDevice>) -> Self {
        Self { inner }
    }
}

impl Ext4BlockDevice for BlockDeviceAdapter {
    fn read_offset(&self, offset: usize) -> Vec<u8> {
        let mut buf = vec![0u8; EXT4_BLOCK_SIZE];
        self.inner.read_bytes(offset, &mut buf).ok();
        buf
    }

    fn write_offset(&self, offset: usize, data: &[u8]) {
        self.inner.write_bytes(offset, data).ok();
    }
}

// ---------------------------------------------------------------------------
// Ext4Fs — FileSystem implementation
// ---------------------------------------------------------------------------

pub struct Ext4Fs {
    fs_id: u64,
    #[allow(dead_code)] // held to keep the Ext4 alive alongside the root inode
    ext4: Arc<Mutex<Ext4>>,
    root: Arc<Ext4FsInode>,
}

impl Ext4Fs {
    pub fn mount(device: Arc<dyn BlockDevice>) -> Result<Self, FsError> {
        let fs_id = super::vfs::allocate_filesystem_id();
        let adapter = Arc::new(BlockDeviceAdapter::new(device));
        let ext4 = Ext4::open(adapter);
        let ext4 = Arc::new(Mutex::new(ext4));

        let root = Ext4FsInode::load_concrete(fs_id, ext4.clone(), EXT4_ROOT_INODE)?;

        Ok(Self { fs_id, ext4, root })
    }
}

impl FileSystem for Ext4Fs {
    fn name(&self) -> &str {
        "ext4"
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }

    fn fs_id(&self) -> u64 {
        self.fs_id
    }
}

// ---------------------------------------------------------------------------
// Ext4FsInode — Inode implementation
// ---------------------------------------------------------------------------

pub struct Ext4FsInode {
    fs_id: u64,
    ext4: Arc<Mutex<Ext4>>,
    inode_num: u32,
    file_type: FileType,
    size: u64,
    uid: u32,
    gid: u32,
    // Stored for future use in readdir DirEntry metadata.
    #[allow(dead_code)]
    atime: u32,
    #[allow(dead_code)]
    mtime: u32,
    #[allow(dead_code)]
    ctime: u32,
}

impl Ext4FsInode {
    fn load_concrete(
        fs_id: u64,
        ext4: Arc<Mutex<Ext4>>,
        inode_num: u32,
    ) -> Result<Arc<Ext4FsInode>, FsError> {
        let attr = ext4
            .lock()
            .fuse_getattr(inode_num as u64)
            .map_err(|_| FsError::NotFound)?;

        Ok(Arc::new(Ext4FsInode {
            fs_id,
            ext4,
            inode_num,
            file_type: map_inode_mode(attr.kind),
            size: attr.size,
            uid: attr.uid,
            gid: attr.gid,
            atime: attr.atime,
            mtime: attr.mtime,
            ctime: attr.ctime,
        }))
    }

    fn load(fs_id: u64, ext4: Arc<Mutex<Ext4>>, inode_num: u32) -> Result<Arc<dyn Inode>, FsError> {
        Self::load_concrete(fs_id, ext4, inode_num).map(|n| n as Arc<dyn Inode>)
    }
}

impl Inode for Ext4FsInode {
    fn ino(&self) -> u64 {
        self.inode_num as u64
    }

    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, FsError> {
        if self.file_type == FileType::Directory {
            return Err(FsError::IsADirectory);
        }
        if buf.is_empty() || offset >= self.size as usize {
            return Ok(0);
        }

        let guard = self.ext4.lock();

        // Fast (inline) symlinks store their target directly in the inode's
        // `block[0..15]` bytes rather than in data blocks, so they have no
        // extent tree. Detect this and serve the data from the raw inode field.
        if self.file_type == FileType::Symlink {
            let iref = guard.get_inode_ref(self.inode_num);
            if iref.inode.flags() & INODE_FLAG_EXTENTS == 0 {
                let block_bytes: &[u8] = unsafe {
                    core::slice::from_raw_parts(iref.inode.block().as_ptr() as *const u8, 60)
                };
                let available = (self.size as usize).saturating_sub(offset);
                let to_copy = core::cmp::min(buf.len(), available);
                buf[..to_copy].copy_from_slice(&block_bytes[offset..offset + to_copy]);
                return Ok(to_copy);
            }
        }

        guard
            .read_at(self.inode_num, offset, buf)
            .map_err(|_| FsError::IoError)
    }

    fn write(&self, offset: usize, buf: &[u8]) -> Result<usize, FsError> {
        if self.file_type == FileType::Directory {
            return Err(FsError::IsADirectory);
        }
        if buf.is_empty() {
            return Ok(0);
        }

        self.ext4
            .lock()
            .write_at(self.inode_num, offset, buf)
            .map_err(|_| FsError::IoError)
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>, FsError> {
        if self.file_type != FileType::Directory {
            return Err(FsError::NotADirectory);
        }

        let attr = self
            .ext4
            .lock()
            .fuse_lookup(self.inode_num as u64, name)
            .map_err(|_| FsError::NotFound)?;

        Ext4FsInode::load(self.fs_id, self.ext4.clone(), attr.ino as u32)
    }

    fn create(&self, name: &str, file_type: FileType) -> Result<Arc<dyn Inode>, FsError> {
        if self.file_type != FileType::Directory {
            return Err(FsError::NotADirectory);
        }

        let inode_mode = match file_type {
            FileType::Regular => InodeFileType::S_IFREG.bits(),
            FileType::Directory => InodeFileType::S_IFDIR.bits(),
            _ => return Err(FsError::NotSupported),
        };

        let child_inode_num = self
            .ext4
            .lock()
            .create(self.inode_num, name, inode_mode)
            .map_err(|_| FsError::IoError)?
            .inode_num;

        Ext4FsInode::load(self.fs_id, self.ext4.clone(), child_inode_num)
    }

    fn file_type(&self) -> FileType {
        self.file_type
    }

    fn size(&self) -> usize {
        self.size as usize
    }

    fn uid(&self) -> u32 {
        self.uid
    }

    fn gid(&self) -> u32 {
        self.gid
    }

    fn filesystem_name(&self) -> &'static str {
        "ext4"
    }

    fn fs_id(&self) -> u64 {
        self.fs_id
    }

    fn readdir(
        &self,
        cursor: &mut DirCursor,
        name_buf: &mut [u8],
        visit: &mut dyn for<'a> FnMut(DirEntry<'a>) -> bool,
    ) -> Result<usize, FsError> {
        if self.file_type != FileType::Directory {
            return Err(FsError::NotADirectory);
        }

        let entries = self.ext4.lock().ext4_dir_get_entries(self.inode_num);
        let mut emitted = 0usize;
        let mut index = cursor.offset as usize;

        while index < entries.len() {
            let raw = &entries[index];

            // Skip unused entries and the synthesised . / .. entries.
            if raw.unused() {
                index += 1;
                continue;
            }
            let name_str = raw.get_name();
            if name_str == "." || name_str == ".." {
                index += 1;
                continue;
            }

            let ft = map_dir_entry_type(raw.get_de_type());
            let name = copy_name_into(name_str.as_str(), name_buf)?;
            let mut out = DirEntry::new(name, ft, raw.inode as u64);
            out.uid = self.uid;
            out.gid = self.gid;
            emitted += 1;
            index += 1;

            if !visit(out) {
                cursor.offset = index as u64;
                return Ok(emitted);
            }
        }

        cursor.offset = index as u64;
        Ok(emitted)
    }
}

// ---------------------------------------------------------------------------
// Ext4FsFactory — mounts ext4 from a block device
// ---------------------------------------------------------------------------

#[cfg(feature = "drivers")]
pub struct Ext4FsFactory;

#[cfg(feature = "drivers")]
impl super::FileSystemFactory for Ext4FsFactory {
    fn mount(
        &self,
        _mountpoint: Arc<dyn Inode>,
        device: Option<super::MountBlockDevice>,
        _options: &super::MountOptions,
    ) -> Result<Arc<dyn FileSystem>, FsError> {
        let block = device.ok_or(FsError::NotFound)?;
        let fs = Ext4Fs::mount(block)?;
        Ok(Arc::new(fs))
    }

    fn unmount(
        &self,
        _mountpoint: Arc<dyn Inode>,
        _device: Option<super::MountBlockDevice>,
    ) -> Result<(), FsError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Ext4FsModule — KernelModule for device-backed mounts
// ---------------------------------------------------------------------------

#[cfg(feature = "drivers")]
pub struct Ext4FsModule {
    module_name: String,
    device: Arc<dyn BlockDevice>,
}

#[cfg(feature = "drivers")]
impl Ext4FsModule {
    pub fn new(module_name: String, device: Arc<dyn BlockDevice>) -> Self {
        Self {
            module_name,
            device,
        }
    }
}

#[cfg(feature = "drivers")]
impl KernelModule for Ext4FsModule {
    fn name(&self) -> &str {
        self.module_name.as_str()
    }

    fn version(&self) -> &str {
        "1.3.3"
    }

    fn description(&self) -> &str {
        "ext4 filesystem"
    }

    fn init(&self, _registry: &dyn KernelRegistry) -> Result<(), ModuleError> {
        let fs = Ext4Fs::mount(Arc::clone(&self.device)).map_err(|_| ModuleError::InitFailed)?;
        crate::fs::register_filesystem(Arc::new(fs)).map_err(|_| ModuleError::InitFailed)
    }

    fn cleanup(&self, _registry: &dyn KernelRegistry) -> Result<(), ModuleError> {
        crate::fs::unregister_filesystem("ext4").map_err(|_| ModuleError::CleanupFailed)
    }
}

#[cfg(feature = "drivers")]
pub fn register_module(
    registry: &dyn KernelRegistry,
    module_name: String,
    device: Arc<dyn BlockDevice>,
) {
    let module: Arc<dyn KernelModule> = Arc::new(Ext4FsModule::new(module_name, device));
    let _ = registry.register_module(module);
}
