extern crate alloc;

#[cfg(feature = "drivers")]
pub mod devfs;
pub mod fat;
pub mod initrd;
#[cfg(feature = "process")]
pub mod procfs;
pub mod ramfs;
pub mod vfs;

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;

use self::ramfs::RamFs;
use self::vfs::{FileSystem, FileType, FsError, Inode};
#[cfg(feature = "drivers")]
use crate::drivers::traits::BlockDevice;
use crate::module::traits::KernelRegistry;
use spin::{Mutex, Once};

#[cfg(feature = "drivers")]
pub type MountBlockDevice = Arc<dyn BlockDevice>;
#[cfg(not(feature = "drivers"))]
pub type MountBlockDevice = ();

pub trait FileSystemFactory: Send + Sync {
    fn mount(
        &self,
        mountpoint: Arc<dyn Inode>,
        device: Option<MountBlockDevice>,
    ) -> Result<Arc<dyn FileSystem>, FsError>;

    fn unmount(
        &self,
        mountpoint: Arc<dyn Inode>,
        device: Option<MountBlockDevice>,
    ) -> Result<(), FsError>;
}

pub fn init() {
    let ramfs: Arc<dyn FileSystem> = Arc::new(RamFs::new());
    let _ = register_filesystem(ramfs);
    log::info!("registered ramfs filesystem");

    #[cfg(feature = "drivers")]
    {
        let _ = register_filesystem_factory("fatfs", Arc::new(fat::FatFsFactory));
        log::info!("registered fatfs filesystem factory");
    }
}

pub fn register_modules(registry: &dyn KernelRegistry) {
    #[cfg(feature = "process")]
    procfs::register_module(registry);

    #[cfg(feature = "drivers")]
    devfs::register_modules(registry);
}

#[derive(Clone)]
pub enum MountDevice {
    None,
    DevicePath(String),
    #[cfg(feature = "drivers")]
    Block(Arc<dyn BlockDevice>),
}

impl MountDevice {
    pub fn device_path(path: &str) -> Self {
        Self::DevicePath(path.to_string())
    }
}

#[derive(Clone, Copy)]
pub struct MountOptions {
    pub create_mountpoint_if_missing: bool,
}

impl Default for MountOptions {
    fn default() -> Self {
        Self {
            create_mountpoint_if_missing: true,
        }
    }
}

#[derive(Clone)]
struct MountSpec {
    path: String,
    filesystem: String,
    device: MountDevice,
    options: MountOptions,
}

#[derive(Default)]
struct FilesystemTable {
    filesystems: BTreeMap<String, Arc<dyn FileSystem>>,
    filesystem_factories: BTreeMap<String, Arc<dyn FileSystemFactory>>,
    registration_counts: BTreeMap<String, usize>,
    active_mounts: BTreeMap<String, usize>,
}

#[derive(Default)]
struct MountTable {
    pending: Vec<MountSpec>,
}

static FILESYSTEM_TABLE: Once<Mutex<FilesystemTable>> = Once::new();
static MOUNT_TABLE: Once<Mutex<MountTable>> = Once::new();

fn filesystem_table() -> &'static Mutex<FilesystemTable> {
    FILESYSTEM_TABLE.call_once(|| Mutex::new(FilesystemTable::default()))
}

fn mount_table() -> &'static Mutex<MountTable> {
    MOUNT_TABLE.call_once(|| Mutex::new(MountTable::default()))
}

pub fn register_filesystem(fs: Arc<dyn FileSystem>) -> Result<(), FsError> {
    let name = fs.name().to_string();
    let mut table = filesystem_table().lock();

    if table.filesystems.contains_key(name.as_str()) {
        let count = table.registration_counts.entry(name).or_insert(1);
        *count = count.saturating_add(1);
        return Ok(());
    }

    table.registration_counts.insert(name.clone(), 1);
    table.filesystems.insert(name, fs);
    Ok(())
}

pub fn filesystem(name: &str) -> Result<Arc<dyn FileSystem>, FsError> {
    filesystem_table()
        .lock()
        .filesystems
        .get(name)
        .cloned()
        .ok_or(FsError::NotFound)
}

pub fn register_filesystem_factory(
    name: &str,
    factory: Arc<dyn FileSystemFactory>,
) -> Result<(), FsError> {
    let normalized = name.trim();
    if normalized.is_empty() {
        return Err(FsError::NotSupported);
    }

    let mut table = filesystem_table().lock();
    if table.filesystem_factories.contains_key(normalized) {
        return Err(FsError::AlreadyExists);
    }

    table
        .filesystem_factories
        .insert(normalized.to_string(), factory);
    Ok(())
}

pub fn unregister_filesystem_factory(name: &str) -> Result<(), FsError> {
    let normalized = name.trim();
    if normalized.is_empty() {
        return Err(FsError::NotSupported);
    }

    let mut table = filesystem_table().lock();
    if table.filesystem_factories.remove(normalized).is_none() {
        return Err(FsError::NotFound);
    }
    Ok(())
}

fn filesystem_from_factory(
    name: &str,
    mountpoint: Arc<dyn Inode>,
    device: Option<MountBlockDevice>,
) -> Result<Arc<dyn FileSystem>, FsError> {
    let factory = {
        let table = filesystem_table().lock();
        table.filesystem_factories.get(name).cloned()
    }
    .ok_or(FsError::NotFound)?;

    factory.mount(mountpoint, device)
}

pub fn unregister_filesystem(name: &str) -> Result<(), FsError> {
    let mut table = filesystem_table().lock();
    if table.active_mounts.get(name).copied().unwrap_or(0) > 0 {
        return Err(FsError::Busy);
    }

    let Some(count) = table.registration_counts.get_mut(name) else {
        return Err(FsError::NotFound);
    };
    if *count > 1 {
        *count -= 1;
        return Ok(());
    }
    table.registration_counts.remove(name);
    if table.filesystems.remove(name).is_none() {
        return Err(FsError::NotFound);
    }

    {
        let mut mounts = mount_table().lock();
        mounts.pending.retain(|entry| entry.filesystem != name);
    }

    Ok(())
}

pub fn register_mount(path: &str, filesystem: &str, device: MountDevice) -> Result<(), FsError> {
    register_mount_with_options(path, filesystem, device, MountOptions::default())
}

pub fn register_mount_with_options(
    path: &str,
    filesystem: &str,
    device: MountDevice,
    options: MountOptions,
) -> Result<(), FsError> {
    let normalized = self::vfs::normalize_absolute_path(path)?;
    let mut table = mount_table().lock();

    if table.pending.iter().any(|entry| entry.path == normalized) {
        return Err(FsError::AlreadyExists);
    }

    table.pending.push(MountSpec {
        path: normalized,
        filesystem: filesystem.to_string(),
        device,
        options,
    });
    Ok(())
}

pub fn mount_registered_filesystems() -> Result<(), FsError> {
    let pending = {
        let mut table = mount_table().lock();
        core::mem::take(&mut table.pending)
    };

    for entry in pending {
        log::info!(
            "mount: attempting filesystem '{}' on '{}'",
            entry.filesystem,
            entry.path
        );

        if matches!(entry.device, MountDevice::None) {
            log::trace!("mount: '{}' requested without explicit device binding", entry.path);
        }

        if entry.path != "/" && entry.options.create_mountpoint_if_missing {
            ensure_directory_path(&entry.path)?;
        }

        let mountpoint = if entry.path == "/" {
            match self::vfs::root_inode() {
                Ok(inode) => Some(inode),
                Err(_) => None,
            }
        } else {
            Some(self::vfs::lookup_path(entry.path.as_str())?)
        };

        let fs = match filesystem(entry.filesystem.as_str()) {
            Ok(fs) => fs,
            Err(FsError::NotFound) => match filesystem_from_factory(
                entry.filesystem.as_str(),
                mountpoint.clone().ok_or(FsError::NotFound)?,
                resolve_mount_block_device(&entry.device)?,
            ) {
                Ok(fs) => fs,
                Err(FsError::NotFound) => {
                    log::warn!(
                        "mount: skipping '{}' -> '{}' (filesystem or device not present)",
                        entry.path,
                        entry.filesystem
                    );
                    continue;
                }
                Err(err) => {
                    log::warn!(
                        "mount: skipping '{}' -> '{}' (mount failed: {:?})",
                        entry.path,
                        entry.filesystem,
                        err
                    );
                    continue;
                }
            },
            Err(err) => return Err(err),
        };
        if entry.path == "/" {
            self::vfs::mount_root(fs)?;
        } else {
            self::vfs::mount(entry.path.as_str(), fs)?;
        }
        track_active_mount(entry.filesystem.as_str());
        log::info!(
            "mount: mounted filesystem '{}' on '{}'",
            entry.filesystem,
            entry.path
        );
    }

    Ok(())
}

fn resolve_mount_block_device(device: &MountDevice) -> Result<Option<MountBlockDevice>, FsError> {
    match device {
        MountDevice::None => Ok(None),
        MountDevice::DevicePath(path) => {
            #[cfg(feature = "drivers")]
            {
                use crate::drivers::registry as driver_registry;

                let name = path.strip_prefix("/dev/").unwrap_or(path.as_str());
                return driver_registry::get_block(name)
                    .map(Some)
                    .ok_or(FsError::NotFound);
            }

            #[cfg(not(feature = "drivers"))]
            {
                let _ = path;
                Err(FsError::NotSupported)
            }
        }
        #[cfg(feature = "drivers")]
        MountDevice::Block(dev) => Ok(Some(Arc::clone(dev))),
    }
}

fn track_active_mount(name: &str) {
    let mut table = filesystem_table().lock();
    let entry = table.active_mounts.entry(name.to_string()).or_insert(0);
    *entry = entry.saturating_add(1);
}

fn ensure_directory_path(path: &str) -> Result<(), FsError> {
    let path = path.trim();
    if path.is_empty() || path == "/" {
        return Ok(());
    }

    let mut current = String::new();
    for segment in path.trim_start_matches('/').split('/') {
        if segment.is_empty() {
            continue;
        }

        current.push('/');
        current.push_str(segment);

        match self::vfs::lookup_path(&current) {
            Ok(inode) if inode.file_type() == FileType::Directory => {}
            Ok(_) => return Err(FsError::NotADirectory),
            Err(FsError::NotFound) => {
                self::vfs::create_path(&current, FileType::Directory)?;
            }
            Err(err) => return Err(err),
        }
    }

    Ok(())
}
