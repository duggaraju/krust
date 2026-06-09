extern crate alloc;

use alloc::{collections::BTreeMap, string::String, sync::Arc, vec::Vec};
use spin::Mutex;

use super::traits::{Device, DeviceType};

#[derive(Clone)]
struct DeviceEntry {
    device: Arc<dyn Device>,
    device_type: DeviceType,
    major: u16,
    minor: u16,
    open_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryError {
    InvalidName,
    NameInUse,
    DeviceNumberInUse,
    Busy,
    NotFound,
}

#[derive(Clone)]
pub struct DeviceDescriptor {
    pub name: String,
    pub device_type: DeviceType,
    pub major: u16,
    pub minor: u16,
    pub device: Arc<dyn Device>,
}

pub struct DeviceRegistry {
    devices: BTreeMap<String, DeviceEntry>,
}

impl DeviceRegistry {
    pub const fn new() -> Self {
        Self {
            devices: BTreeMap::new(),
        }
    }

    pub fn register(
        &mut self,
        name: &str,
        device: Arc<dyn Device>,
        major: u16,
        minor: u16,
    ) -> Result<(), RegistryError> {
        if name.trim().is_empty() {
            return Err(RegistryError::InvalidName);
        }
        if self.devices.contains_key(name) {
            return Err(RegistryError::NameInUse);
        }
        let device_type = device.device_type();
        if self.devices.values().any(|entry| {
            entry.device_type == device_type && entry.major == major && entry.minor == minor
        }) {
            return Err(RegistryError::DeviceNumberInUse);
        }

        self.devices.insert(
            String::from(name),
            DeviceEntry {
                device,
                device_type,
                major,
                minor,
                open_count: 0,
            },
        );
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Device>> {
        self.devices
            .get(name)
            .map(|entry| Arc::clone(&entry.device))
    }

    pub fn get_descriptor(&self, name: &str) -> Option<DeviceDescriptor> {
        self.devices.get(name).map(|entry| DeviceDescriptor {
            name: String::from(name),
            device_type: entry.device_type,
            major: entry.major,
            minor: entry.minor,
            device: Arc::clone(&entry.device),
        })
    }

    pub fn open(&mut self, name: &str) -> Result<DeviceDescriptor, RegistryError> {
        let entry = self.devices.get_mut(name).ok_or(RegistryError::NotFound)?;
        entry.open_count = entry.open_count.saturating_add(1);
        Ok(DeviceDescriptor {
            name: String::from(name),
            device_type: entry.device_type,
            major: entry.major,
            minor: entry.minor,
            device: Arc::clone(&entry.device),
        })
    }

    pub fn close(&mut self, name: &str) -> Result<(), RegistryError> {
        let entry = self.devices.get_mut(name).ok_or(RegistryError::NotFound)?;
        if entry.open_count == 0 {
            return Err(RegistryError::NotFound);
        }
        entry.open_count -= 1;
        Ok(())
    }

    pub fn unregister(&mut self, name: &str) -> Result<(), RegistryError> {
        let Some(entry) = self.devices.get(name) else {
            return Err(RegistryError::NotFound);
        };
        if entry.open_count > 0 {
            return Err(RegistryError::Busy);
        }
        self.devices.remove(name);
        Ok(())
    }

    pub fn get_descriptor_by_number(
        &self,
        device_type: DeviceType,
        major: u16,
        minor: u16,
    ) -> Option<DeviceDescriptor> {
        self.devices.iter().find_map(|(name, entry)| {
            if entry.device_type == device_type && entry.major == major && entry.minor == minor {
                Some(DeviceDescriptor {
                    name: name.clone(),
                    device_type: entry.device_type,
                    major: entry.major,
                    minor: entry.minor,
                    device: Arc::clone(&entry.device),
                })
            } else {
                None
            }
        })
    }

    pub fn list(&self) -> Vec<String> {
        self.devices.keys().cloned().collect()
    }

    pub fn list_descriptors(&self) -> Vec<DeviceDescriptor> {
        self.devices
            .iter()
            .map(|(name, entry)| DeviceDescriptor {
                name: name.clone(),
                device_type: entry.device_type,
                major: entry.major,
                minor: entry.minor,
                device: Arc::clone(&entry.device),
            })
            .collect()
    }

    pub fn list_by_prefix(&self, prefix: &str) -> Vec<DeviceDescriptor> {
        self.devices
            .iter()
            .filter(|(name, _)| name.starts_with(prefix))
            .map(|(name, entry)| DeviceDescriptor {
                name: name.clone(),
                device_type: entry.device_type,
                major: entry.major,
                minor: entry.minor,
                device: Arc::clone(&entry.device),
            })
            .collect()
    }
}

static REGISTRY: Mutex<DeviceRegistry> = Mutex::new(DeviceRegistry::new());

pub fn register(
    name: &str,
    device: Arc<dyn Device>,
    major: u16,
    minor: u16,
) -> Result<(), RegistryError> {
    REGISTRY.lock().register(name, device, major, minor)
}

pub fn get(name: &str) -> Option<Arc<dyn Device>> {
    REGISTRY.lock().get(name)
}

pub fn get_descriptor(name: &str) -> Option<DeviceDescriptor> {
    REGISTRY.lock().get_descriptor(name)
}

pub fn open(name: &str) -> Result<DeviceDescriptor, RegistryError> {
    REGISTRY.lock().open(name)
}

pub fn close(name: &str) -> Result<(), RegistryError> {
    REGISTRY.lock().close(name)
}

pub fn unregister(name: &str) -> Result<(), RegistryError> {
    REGISTRY.lock().unregister(name)
}

pub fn get_descriptor_by_number(
    device_type: DeviceType,
    major: u16,
    minor: u16,
) -> Option<DeviceDescriptor> {
    REGISTRY
        .lock()
        .get_descriptor_by_number(device_type, major, minor)
}

pub fn list() -> Vec<String> {
    REGISTRY.lock().list()
}

pub fn list_descriptors() -> Vec<DeviceDescriptor> {
    REGISTRY.lock().list_descriptors()
}

pub fn list_by_prefix(prefix: &str) -> Vec<DeviceDescriptor> {
    REGISTRY.lock().list_by_prefix(prefix)
}
