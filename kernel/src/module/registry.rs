extern crate alloc;

use alloc::{
    boxed::Box,
    collections::BTreeMap,
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use log::{error, info, warn};
use spin::Mutex;

use super::traits::{KernelModule, ModuleError};

pub struct ModuleRegistry {
    loaded_modules: BTreeMap<String, Arc<dyn KernelModule>>,
}

impl ModuleRegistry {
    pub const fn new() -> Self {
        Self {
            loaded_modules: BTreeMap::new(),
        }
    }

    pub fn load(
        &mut self,
        module: Arc<dyn KernelModule>,
        registry: &dyn super::traits::KernelRegistry,
    ) -> Result<(), ModuleError> {
        let name = module.name();

        if name.is_empty() || module.version().is_empty() {
            warn!("refusing to load invalid module metadata");
            return Err(ModuleError::InvalidModule);
        }

        if self.loaded_modules.contains_key(name) {
            warn!("module '{}' is already loaded", name);
            return Err(ModuleError::AlreadyLoaded);
        }

        for dependency in module.dependencies() {
            if !self.loaded_modules.contains_key(*dependency) {
                error!("module '{}' is missing dependency '{}'", name, dependency);
                return Err(ModuleError::DependencyMissing(leak_dependency_name(
                    dependency,
                )));
            }
        }

        module.init(registry)?;
        self.loaded_modules
            .insert(name.to_string(), Arc::clone(&module));
        info!(
            "loaded module '{}' v{} ({})",
            name,
            module.version(),
            module.description()
        );
        Ok(())
    }

    pub fn unload(&mut self, name: &str) -> Result<(), ModuleError> {
        let Some(module) = self.loaded_modules.get(name).cloned() else {
            warn!("module '{}' is not loaded", name);
            return Err(ModuleError::NotLoaded);
        };

        if let Some(dependent) = self.find_dependent(name) {
            error!(
                "cannot unload module '{}' because '{}' depends on it",
                name, dependent
            );
            return Err(ModuleError::CleanupFailed);
        }

        let registry = super::kernel_services();
        module.cleanup(&registry)?;
        self.loaded_modules.remove(name);
        info!("unloaded module '{}'", name);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn KernelModule>> {
        self.loaded_modules.get(name).cloned()
    }

    pub fn loaded_modules(&self) -> Vec<&str> {
        self.loaded_modules.keys().map(String::as_str).collect()
    }

    pub fn loaded_modules_with_info(&self) -> Vec<(&str, &str, &str)> {
        self.loaded_modules
            .iter()
            .map(|(name, module)| (name.as_str(), module.version(), module.description()))
            .collect()
    }

    fn find_dependent(&self, name: &str) -> Option<&str> {
        self.loaded_modules
            .iter()
            .find_map(|(module_name, module)| {
                module
                    .dependencies()
                    .iter()
                    .any(|dependency| *dependency == name)
                    .then_some(module_name.as_str())
            })
    }
}

impl Default for ModuleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub static MODULE_REGISTRY: Mutex<ModuleRegistry> = Mutex::new(ModuleRegistry::new());

pub fn init() {
    let mut registry = MODULE_REGISTRY.lock();
    *registry = ModuleRegistry::new();
    info!("module registry initialized");
}

fn leak_dependency_name(name: &str) -> &'static str {
    Box::leak(name.to_string().into_boxed_str())
}
