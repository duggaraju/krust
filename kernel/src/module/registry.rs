extern crate alloc;

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use log::{error, info, warn};
use spin::Mutex;

use super::traits::{KernelModule, ModuleError};

pub struct ModuleRegistry {
    loaded_modules: BTreeMap<String, Arc<dyn KernelModule>>,
    known_modules: BTreeMap<String, Arc<dyn KernelModule>>,
    loading_modules: BTreeSet<String>,
    failed_modules: BTreeSet<String>,
}

impl ModuleRegistry {
    pub const fn new() -> Self {
        Self {
            loaded_modules: BTreeMap::new(),
            known_modules: BTreeMap::new(),
            loading_modules: BTreeSet::new(),
            failed_modules: BTreeSet::new(),
        }
    }

    pub fn register_known(&mut self, module: Arc<dyn KernelModule>) -> Result<(), ModuleError> {
        let name = module.name();

        if name.is_empty() || module.version().is_empty() {
            warn!("refusing to register invalid module metadata");
            return Err(ModuleError::InvalidModule);
        }

        if self.loaded_modules.contains_key(name) {
            warn!("module '{}' is already loaded", name);
            return Err(ModuleError::AlreadyLoaded);
        }

        // Clear any previous failure so callers can retry a failed module by
        // submitting a fresh Arc (e.g. after fixing a device state).
        if self.failed_modules.remove(name) {
            info!(
                "module '{}' previously failed; clearing failure state for retry",
                name
            );
        }

        self.known_modules
            .insert(name.to_string(), Arc::clone(&module));
        Ok(())
    }

    pub fn known(&self, name: &str) -> Option<Arc<dyn KernelModule>> {
        self.known_modules.get(name).cloned()
    }

    pub fn is_loaded(&self, name: &str) -> bool {
        self.loaded_modules.contains_key(name)
    }

    pub fn is_loading(&self, name: &str) -> bool {
        self.loading_modules.contains(name)
    }

    pub fn has_failed(&self, name: &str) -> bool {
        self.failed_modules.contains(name)
    }

    pub fn mark_loading(&mut self, name: &str) {
        self.loading_modules.insert(name.to_string());
    }

    pub fn clear_loading(&mut self, name: &str) {
        self.loading_modules.remove(name);
    }

    pub fn mark_failed(&mut self, name: &str) {
        self.failed_modules.insert(name.to_string());
    }

    pub fn clear_failed(&mut self, name: &str) {
        self.failed_modules.remove(name);
    }

    pub fn mark_loaded(&mut self, module: Arc<dyn KernelModule>) {
        let name = module.name().to_string();
        self.loaded_modules.insert(name.clone(), module);
        self.failed_modules.remove(&name);
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
