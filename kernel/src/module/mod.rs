extern crate alloc;

use alloc::sync::Arc;

pub mod registry;
pub mod traits;

pub(crate) struct KernelServices;

pub(crate) fn kernel_services() -> KernelServices {
    KernelServices
}

impl traits::KernelRegistry for KernelServices {
    fn register_module(
        &self,
        module: Arc<dyn traits::KernelModule>,
    ) -> Result<(), traits::ModuleError> {
        registry::MODULE_REGISTRY.lock().load(module, self)
    }

    fn unregister_module(&self, name: &str) -> Result<(), traits::ModuleError> {
        registry::MODULE_REGISTRY.lock().unload(name)
    }

    fn register_filesystem(
        &self,
        fs: Arc<dyn crate::fs::vfs::FileSystem>,
    ) -> Result<(), traits::ModuleError> {
        crate::fs::register_filesystem(fs).map_err(|_| traits::ModuleError::InitFailed)
    }

    fn unregister_filesystem(&self, name: &str) -> Result<(), traits::ModuleError> {
        crate::fs::unregister_filesystem(name).map_err(|_| traits::ModuleError::CleanupFailed)
    }

    #[cfg(feature = "drivers")]
    fn register_device(
        &self,
        name: &str,
        device: Arc<dyn crate::drivers::traits::Device>,
        major: u16,
        minor: u16,
    ) -> Result<(), traits::ModuleError> {
        crate::drivers::registry::register(name, device, major, minor)
            .map_err(|_| traits::ModuleError::InitFailed)
    }

    fn unregister_device(&self, name: &str) -> Result<(), traits::ModuleError> {
        crate::drivers::registry::unregister(name).map_err(|_| traits::ModuleError::CleanupFailed)
    }
}

pub fn init() {
    registry::init();
    let services = kernel_services();

    #[cfg(feature = "fs")]
    crate::fs::register_modules(&services);

    #[cfg(feature = "drivers")]
    crate::drivers::register_modules(&services);
}
