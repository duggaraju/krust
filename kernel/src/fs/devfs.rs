extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;

use crate::drivers::registry;
use crate::drivers::traits::{DeviceError, DeviceType};
use crate::module::traits::{KernelModule, KernelRegistry, ModuleError};

use super::vfs::{DirCursor, DirEntry, FileSystem, FileType, FsError, Inode, copy_name_into};

pub struct DevFs;

pub struct DevFsModule;

impl DevFs {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DevFs {
    fn default() -> Self {
        Self::new()
    }
}

impl KernelModule for DevFsModule {
    fn name(&self) -> &str {
        "devfs"
    }

    fn version(&self) -> &str {
        "0.1.0"
    }

    fn description(&self) -> &str {
        "Device filesystem"
    }

    fn init(&self, _registry: &dyn KernelRegistry) -> Result<(), ModuleError> {
        crate::fs::register_filesystem(Arc::new(DevFs::new()))
            .map_err(|_| ModuleError::InitFailed)
    }

    fn cleanup(&self, _registry: &dyn KernelRegistry) -> Result<(), ModuleError> {
        crate::fs::unregister_filesystem("dev").map_err(|_| ModuleError::CleanupFailed)
    }
}

pub fn register_modules(registry: &dyn KernelRegistry) {
    let _ = registry.register_module(Arc::new(DevFsModule));
}

impl FileSystem for DevFs {
    fn name(&self) -> &str {
        "dev"
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        Arc::new(DevRootInode)
    }
}

struct DevRootInode;

struct DevNodeInode {
    name: String,
    file_type: FileType,
    major: u16,
    minor: u16,
}

/// Virtual directory inode for `/dev/pts` that lists all registered `pts*` devices.
struct DevPtsDirInode;

/// A single entry inside `/dev/pts`.
struct DevPtsNodeInode {
    major: u16,
    minor: u16,
}

impl Inode for DevRootInode {
    fn ino(&self) -> u64 {
        DEV_ROOT_INO
    }

    fn read(&self, _offset: usize, _buf: &mut [u8]) -> Result<usize, FsError> {
        Err(FsError::IsADirectory)
    }

    fn write(&self, _offset: usize, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::IsADirectory)
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>, FsError> {
        if name == "pts" {
            return Ok(Arc::new(DevPtsDirInode));
        }
        let descriptor = registry::get_descriptor(name).ok_or(FsError::NotFound)?;
        Ok(Arc::new(DevNodeInode {
            name: descriptor.name,
            file_type: to_file_type(descriptor.device_type),
            major: descriptor.major,
            minor: descriptor.minor,
        }))
    }

    fn create(&self, _name: &str, _file_type: FileType) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotSupported)
    }

    fn file_type(&self) -> FileType {
        FileType::Directory
    }

    fn size(&self) -> usize {
        // +1 for the "pts" subdirectory entry
        registry::list_descriptors().len() + 1
    }

    fn filesystem_name(&self) -> &'static str {
        "dev"
    }

    fn readdir(
        &self,
        cursor: &mut DirCursor,
        name_buf: &mut [u8],
        visit: &mut dyn for<'a> FnMut(DirEntry<'a>) -> bool,
    ) -> Result<usize, FsError> {
        let descriptors = registry::list_descriptors();
        let mut emitted = 0usize;
        let mut index = cursor.offset as usize;

        while index < descriptors.len() {
            let descriptor = &descriptors[index];
            let name = copy_name_into(descriptor.name.as_str(), name_buf)?;
            let entry = DirEntry::new(
                name,
                to_file_type(descriptor.device_type),
                device_inode_number(descriptor.name.as_str()),
            );
            emitted += 1;
            index += 1;
            if !visit(entry) {
                cursor.offset = index as u64;
                return Ok(emitted);
            }
        }

        // Emit "pts" directory entry after all flat device entries.
        if index == descriptors.len() {
            let name = copy_name_into("pts", name_buf)?;
            emitted += 1;
            index += 1;
            if !visit(DirEntry::new(name, FileType::Directory, DEV_PTS_DIR_INO)) {
                cursor.offset = index as u64;
                return Ok(emitted);
            }
        }

        cursor.offset = index as u64;
        Ok(emitted)
    }
}

impl Inode for DevNodeInode {
    fn ino(&self) -> u64 {
        device_inode_number(&self.name)
    }

    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, FsError> {
        let descriptor = lookup_device(self.file_type, self.major, self.minor)?;
        descriptor
            .device
            .read(offset, buf)
            .map_err(fs_error_from_device)
    }

    fn write(&self, offset: usize, buf: &[u8]) -> Result<usize, FsError> {
        let descriptor = lookup_device(self.file_type, self.major, self.minor)?;
        descriptor
            .device
            .write(offset, buf)
            .map_err(fs_error_from_device)
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn create(&self, _name: &str, _file_type: FileType) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn file_type(&self) -> FileType {
        self.file_type
    }

    fn size(&self) -> usize {
        0
    }

    fn filesystem_name(&self) -> &'static str {
        "dev"
    }

    fn device_numbers(&self) -> Option<(u16, u16)> {
        Some((self.major, self.minor))
    }
}

fn to_file_type(device_type: DeviceType) -> FileType {
    match device_type {
        DeviceType::Char => FileType::CharDevice,
        DeviceType::Block => FileType::BlockDevice,
        DeviceType::Network => FileType::CharDevice,
    }
}

fn device_inode_number(name: &str) -> u64 {
    let mut hash = 0u64;
    for byte in name.bytes() {
        hash = hash.wrapping_mul(131).wrapping_add(u64::from(byte));
    }
    DEV_NODE_BASE_INO.wrapping_add(hash)
}

fn lookup_device(
    file_type: FileType,
    major: u16,
    minor: u16,
) -> Result<registry::DeviceDescriptor, FsError> {
    let device_type = match file_type {
        FileType::CharDevice => DeviceType::Char,
        FileType::BlockDevice => DeviceType::Block,
        _ => return Err(FsError::NotSupported),
    };
    registry::get_descriptor_by_number(device_type, major, minor).ok_or(FsError::NotFound)
}

fn fs_error_from_device(err: DeviceError) -> FsError {
    match err {
        DeviceError::IoError | DeviceError::NotReady => FsError::IoError,
        DeviceError::InvalidArgument | DeviceError::NotFound => FsError::NotFound,
        DeviceError::Busy => FsError::Busy,
        DeviceError::NotSupported => FsError::NotSupported,
    }
}

impl Inode for DevPtsDirInode {
    fn ino(&self) -> u64 {
        DEV_PTS_DIR_INO
    }

    fn read(&self, _offset: usize, _buf: &mut [u8]) -> Result<usize, FsError> {
        Err(FsError::IsADirectory)
    }

    fn write(&self, _offset: usize, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::IsADirectory)
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>, FsError> {
        // Entries are named "pts0", "pts1", … exactly matching the registry key.
        let descriptor = registry::get_descriptor(name).ok_or(FsError::NotFound)?;
        // Only expose pts* devices here.
        if !descriptor.name.starts_with("pts") {
            return Err(FsError::NotFound);
        }
        Ok(Arc::new(DevPtsNodeInode {
            major: descriptor.major,
            minor: descriptor.minor,
        }))
    }

    fn create(&self, _name: &str, _file_type: FileType) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotSupported)
    }

    fn file_type(&self) -> FileType {
        FileType::Directory
    }

    fn size(&self) -> usize {
        registry::list_by_prefix("pts").len()
    }

    fn filesystem_name(&self) -> &'static str {
        "dev"
    }

    fn readdir(
        &self,
        cursor: &mut DirCursor,
        name_buf: &mut [u8],
        visit: &mut dyn for<'a> FnMut(DirEntry<'a>) -> bool,
    ) -> Result<usize, FsError> {
        let pts_devices = registry::list_by_prefix("pts");
        let mut emitted = 0usize;
        let mut index = cursor.offset as usize;

        while index < pts_devices.len() {
            let descriptor = &pts_devices[index];
            let name = copy_name_into(descriptor.name.as_str(), name_buf)?;
            let ino = DEV_PTS_NODE_BASE_INO.wrapping_add(descriptor.minor as u64);
            emitted += 1;
            index += 1;
            if !visit(DirEntry::new(name, FileType::CharDevice, ino)) {
                cursor.offset = index as u64;
                return Ok(emitted);
            }
        }

        cursor.offset = index as u64;
        Ok(emitted)
    }
}

impl Inode for DevPtsNodeInode {
    fn ino(&self) -> u64 {
        DEV_PTS_NODE_BASE_INO.wrapping_add(self.minor as u64)
    }

    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, FsError> {
        let descriptor = registry::get_descriptor_by_number(DeviceType::Char, self.major, self.minor)
            .ok_or(FsError::NotFound)?;
        descriptor.device.read(offset, buf).map_err(fs_error_from_device)
    }

    fn write(&self, offset: usize, buf: &[u8]) -> Result<usize, FsError> {
        let descriptor = registry::get_descriptor_by_number(DeviceType::Char, self.major, self.minor)
            .ok_or(FsError::NotFound)?;
        descriptor.device.write(offset, buf).map_err(fs_error_from_device)
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn create(&self, _name: &str, _file_type: FileType) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn file_type(&self) -> FileType {
        FileType::CharDevice
    }

    fn size(&self) -> usize {
        0
    }

    fn filesystem_name(&self) -> &'static str {
        "dev"
    }

    fn device_numbers(&self) -> Option<(u16, u16)> {
        Some((self.major, self.minor))
    }
}

const DEV_ROOT_INO: u64 = 2_000;
const DEV_PTS_DIR_INO: u64 = 2_001;
const DEV_NODE_BASE_INO: u64 = 20_000;
const DEV_PTS_NODE_BASE_INO: u64 = 30_000;
