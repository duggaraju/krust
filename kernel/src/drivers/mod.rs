extern crate alloc;

pub mod registry;
pub mod serial;
pub mod traits;
pub mod video;

use alloc::sync::Arc;

pub fn init() {
    let serial = Arc::new(serial::init());
    registry::register("serial0", serial);
}
