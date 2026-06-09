extern crate alloc;

pub mod fat;
pub mod initrd;
pub mod ramfs;
pub mod vfs;

use alloc::sync::Arc;

use self::ramfs::RamFs;
use self::vfs::FileSystem;

pub fn init() {
    let ramfs: Arc<dyn FileSystem> = Arc::new(RamFs::new());

    if self::vfs::mount_root(ramfs).is_ok() {
        log::info!("mounted ramfs at /");
    }
}
