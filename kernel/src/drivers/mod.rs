extern crate alloc;

pub mod block;
pub mod bus;
pub mod char;
pub mod registry;
pub mod traits;
pub mod tty;
pub mod video;

// Compatibility re-exports for existing call sites.
pub use bus::pci;
pub use char::{ldisc, null, pty, serial};

use crate::module::traits::KernelRegistry;
use alloc::sync::Arc;
use log::warn;

pub fn init(virtual_console_count: usize) {
    tty::init_core(virtual_console_count);

    // Register ttyS0 (COM1) and ttyS1 (COM2) — major 4, minor 64/65 (Linux convention).
    for port_num in 1u8..=2 {
        if let Some(dev) = serial::make(port_num) {
            let name = alloc::format!("ttyS{}", port_num - 1);
            let minor = 64u16 + (port_num as u16 - 1);
            let dev = alloc::sync::Arc::new(dev);
            if let Err(err) = registry::register(&name, dev, 4, minor) {
                warn!("failed to register {}: {:?}", name, err);
            }
        }
    }

    for index in 0..virtual_console_count {
        let dev = Arc::new(tty::VirtualConsoleDevice::new(index));
        let name = alloc::format!("tty{}", index);
        if let Err(err) = registry::register(&name, dev, 4, index as u16) {
            warn!("failed to register {}: {:?}", name, err);
        }
    }

    let console = Arc::new(tty::TtyDevice::new_console());
    if let Err(err) = registry::register("console", console, 5, 1) {
        warn!("failed to register console: {:?}", err);
    }

    let null = Arc::new(null::NullDevice::new());
    if let Err(err) = registry::register("null", null, 1, 3) {
        warn!("failed to register null: {:?}", err);
    }

    // ptmx — pseudo-terminal multiplexer (major 5, minor 2).
    // Callers use pty::alloc_pty() to allocate a new PTY pair.
    let ptmx = Arc::new(null::NullDevice::new()); // placeholder; alloc is via pty::alloc_pty()
    if let Err(err) = registry::register("ptmx", ptmx, 5, 2) {
        warn!("failed to register ptmx: {:?}", err);
    }
}

pub fn register_modules(registry: &dyn KernelRegistry) {
    let _ = registry.register_module(Arc::new(video::FrameBufferDeviceModule));

    #[cfg(target_arch = "x86_64")]
    let _ = registry.register_module(Arc::new(pci::PciBusModule));

    #[cfg(target_arch = "x86_64")]
    let _ = registry.register_module(Arc::new(block::sata::SataDeviceModule::new()));
}
