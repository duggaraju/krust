extern crate alloc;

use alloc::{
    boxed::Box,
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use log::{error, info};

pub mod registry;
pub mod traits;

pub(crate) struct KernelServices;

pub(crate) fn kernel_services() -> KernelServices {
    KernelServices
}

impl KernelServices {
    fn load_module_recursive(
        &self,
        name: &str,
        stack: &mut Vec<String>,
    ) -> Result<(), traits::ModuleError> {
        let module = {
            let mut registry = registry::MODULE_REGISTRY.lock();

            if registry.is_loaded(name) {
                return Ok(());
            }

            if registry.has_failed(name) {
                error!("module '{}' previously failed to load", name);
                return Err(traits::ModuleError::InitFailed);
            }

            if registry.is_loading(name) {
                error!(
                    "circular module dependency detected while loading '{}'",
                    name
                );
                return Err(traits::ModuleError::InitFailed);
            }

            let Some(module) = registry.known(name) else {
                error!("missing dependency module '{}'", name);
                return Err(traits::ModuleError::DependencyMissing(
                    leak_dependency_name(name),
                ));
            };

            registry.mark_loading(name);
            module
        };

        stack.push(name.to_string());

        for dependency in module.dependencies() {
            if let Err(err) = self.load_module_recursive(dependency, stack) {
                let mut registry = registry::MODULE_REGISTRY.lock();
                registry.clear_loading(name);
                registry.mark_failed(name);
                error!(
                    "failed to load module '{}' because dependency '{}' failed: {:?}",
                    name, dependency, err
                );
                stack.pop();
                return Err(err);
            }
        }

        match module.init(self) {
            Ok(()) => {
                let mut registry = registry::MODULE_REGISTRY.lock();
                registry.clear_loading(name);
                registry.mark_loaded(Arc::clone(&module));
                info!(
                    "loaded module '{}' v{} ({})",
                    module.name(),
                    module.version(),
                    module.description()
                );
                stack.pop();
                Ok(())
            }
            Err(err) => {
                let mut registry = registry::MODULE_REGISTRY.lock();
                registry.clear_loading(name);
                registry.mark_failed(name);
                error!("failed to initialize module '{}': {:?}", name, err);
                stack.pop();
                Err(err)
            }
        }
    }
}

fn leak_dependency_name(name: &str) -> &'static str {
    Box::leak(name.to_string().into_boxed_str())
}

impl traits::KernelRegistry for KernelServices {
    fn register_module(
        &self,
        module: Arc<dyn traits::KernelModule>,
    ) -> Result<(), traits::ModuleError> {
        let name = module.name().to_string();

        {
            let mut registry = registry::MODULE_REGISTRY.lock();
            registry.register_known(module)?;
        }

        self.load_module_recursive(&name, &mut Vec::new())
    }

    fn unregister_module(&self, name: &str) -> Result<(), traits::ModuleError> {
        registry::MODULE_REGISTRY.lock().unload(name)
    }

    fn register_filesystem_factory(
        &self,
        name: &str,
        factory: Arc<dyn crate::fs::FileSystemFactory>,
    ) -> Result<(), traits::ModuleError> {
        crate::fs::register_filesystem_factory(name, factory)
            .map_err(|_| traits::ModuleError::InitFailed)
    }

    fn unregister_filesystem_factory(&self, name: &str) -> Result<(), traits::ModuleError> {
        crate::fs::unregister_filesystem_factory(name)
            .map_err(|_| traits::ModuleError::CleanupFailed)
    }

    #[cfg(feature = "drivers")]
    fn register_bus(
        &self,
        bus: Arc<dyn crate::drivers::traits::Bus>,
    ) -> Result<(), traits::ModuleError> {
        crate::drivers::registry::register_bus(bus).map_err(|_| traits::ModuleError::InitFailed)
    }

    #[cfg(feature = "drivers")]
    fn unregister_bus(&self, name: &str) -> Result<(), traits::ModuleError> {
        crate::drivers::registry::unregister_bus(name)
            .map_err(|_| traits::ModuleError::CleanupFailed)
    }

    #[cfg(feature = "process")]
    fn register_binfmt_handler(
        &self,
        handler: Arc<dyn crate::process::binfmt::BinaryFormatHandler>,
    ) -> Result<(), traits::ModuleError> {
        crate::process::binfmt::register_handler(handler)
            .map_err(|_| traits::ModuleError::InitFailed)
    }

    #[cfg(feature = "process")]
    fn unregister_binfmt_handler(&self, name: &str) -> Result<(), traits::ModuleError> {
        crate::process::binfmt::unregister_handler(name)
            .map_err(|_| traits::ModuleError::CleanupFailed)
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
    #[cfg(feature = "process")]
    crate::process::binfmt::reset_dynamic_handlers();
    let services = kernel_services();

    #[cfg(feature = "fs")]
    crate::fs::register_modules(&services);

    #[cfg(feature = "drivers")]
    crate::drivers::register_modules(&services);
}
