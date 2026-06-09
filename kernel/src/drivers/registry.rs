extern crate alloc;

use alloc::{collections::BTreeMap, string::String, sync::Arc, vec::Vec};
use spin::Mutex;

use super::traits::Device;

pub struct DeviceRegistry {
    devices: BTreeMap<String, Arc<dyn Device>>,
}

impl DeviceRegistry {
    pub const fn new() -> Self {
        Self {
            devices: BTreeMap::new(),
        }
    }

    pub fn register(&mut self, name: &str, device: Arc<dyn Device>) {
        self.devices.insert(String::from(name), device);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Device>> {
        self.devices.get(name).cloned()
    }

    pub fn list(&self) -> Vec<String> {
        self.devices.keys().cloned().collect()
    }
}

static REGISTRY: Mutex<DeviceRegistry> = Mutex::new(DeviceRegistry::new());

pub fn register(name: &str, device: Arc<dyn Device>) {
    REGISTRY.lock().register(name, device);
}

pub fn get(name: &str) -> Option<Arc<dyn Device>> {
    REGISTRY.lock().get(name)
}

pub fn list() -> Vec<String> {
    REGISTRY.lock().list()
}
