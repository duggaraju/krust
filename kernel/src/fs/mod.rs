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
use self::vfs::{FileSystem, FileType, FsError};
#[cfg(feature = "drivers")]
use crate::drivers::traits::BlockDevice;
use crate::module::traits::KernelRegistry;
use spin::{Mutex, Once};

pub fn init() {
    let ramfs: Arc<dyn FileSystem> = Arc::new(RamFs::new());
    let _ = register_filesystem(ramfs);
    log::info!("registered ramfs filesystem");
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
    #[cfg(feature = "drivers")]
    Block(Arc<dyn BlockDevice>),
}

#[derive(Clone)]
struct MountSpec {
    path: String,
    filesystem: String,
    device: MountDevice,
}

#[derive(Default)]
struct FilesystemTable {
    filesystems: BTreeMap<String, Arc<dyn FileSystem>>,
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
    let normalized = self::vfs::normalize_absolute_path(path)?;
    let mut table = mount_table().lock();

    if table.pending.iter().any(|entry| entry.path == normalized) {
        return Err(FsError::AlreadyExists);
    }

    table.pending.push(MountSpec {
        path: normalized,
        filesystem: filesystem.to_string(),
        device,
    });
    Ok(())
}

pub fn mount_registered_filesystems() -> Result<(), FsError> {
    let pending = {
        let mut table = mount_table().lock();
        core::mem::take(&mut table.pending)
    };

    for entry in pending {
        let _ = &entry.device;
        let fs = filesystem(entry.filesystem.as_str())?;
        if entry.path == "/" {
            self::vfs::mount_root(fs)?;
        } else {
            ensure_directory_path(&entry.path)?;
            self::vfs::mount(entry.path.as_str(), fs)?;
        }
        track_active_mount(entry.filesystem.as_str());
    }

    Ok(())
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
