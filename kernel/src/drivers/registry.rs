extern crate alloc;

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use spin::Mutex;

use super::traits::{BlockDevice, Bus, BusDeviceInfo, BusError, Device, DeviceType};

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
    block_devices: BTreeMap<String, Arc<dyn BlockDevice>>,
}

struct BusEntry {
    bus: Arc<dyn Bus>,
    devices: Vec<BusDeviceInfo>,
}

pub struct BusRegistry {
    buses: BTreeMap<String, BusEntry>,
}

impl DeviceRegistry {
    pub const fn new() -> Self {
        Self {
            devices: BTreeMap::new(),
            block_devices: BTreeMap::new(),
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
        self.block_devices.remove(name);
        Ok(())
    }

    pub fn register_block_device_alias(
        &mut self,
        name: &str,
        device: Arc<dyn BlockDevice>,
    ) -> Result<(), RegistryError> {
        if !self.devices.contains_key(name) {
            return Err(RegistryError::NotFound);
        }
        self.block_devices.insert(name.to_string(), device);
        Ok(())
    }

    pub fn get_block(&self, name: &str) -> Option<Arc<dyn BlockDevice>> {
        self.block_devices.get(name).cloned()
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

impl BusRegistry {
    pub const fn new() -> Self {
        Self {
            buses: BTreeMap::new(),
        }
    }

    pub fn register(&mut self, bus: Arc<dyn Bus>) -> Result<(), BusError> {
        let name = bus.name();
        if name.trim().is_empty() {
            return Err(BusError::InvalidName);
        }
        if self.buses.contains_key(name) {
            return Err(BusError::AlreadyRegistered);
        }

        let devices = bus.enumerate().map_err(|_| BusError::EnumerationFailed)?;

        self.buses
            .insert(name.to_string(), BusEntry { bus, devices });
        Ok(())
    }

    pub fn unregister(&mut self, name: &str) -> Result<(), BusError> {
        if !self.buses.contains_key(name) {
            return Err(BusError::NotFound);
        }
        self.buses.remove(name);
        Ok(())
    }

    pub fn list(&self) -> Vec<String> {
        self.buses.keys().cloned().collect()
    }

    pub fn list_devices(&self, name: &str) -> Option<Vec<BusDeviceInfo>> {
        self.buses.get(name).map(|entry| {
            let _ = entry.bus.name();
            entry.devices.clone()
        })
    }
}

static REGISTRY: Mutex<DeviceRegistry> = Mutex::new(DeviceRegistry::new());
static BUS_REGISTRY: Mutex<BusRegistry> = Mutex::new(BusRegistry::new());

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

pub fn register_block_device_alias(
    name: &str,
    device: Arc<dyn BlockDevice>,
) -> Result<(), RegistryError> {
    REGISTRY.lock().register_block_device_alias(name, device)
}

pub fn get_block(name: &str) -> Option<Arc<dyn BlockDevice>> {
    REGISTRY.lock().get_block(name)
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

pub fn register_bus(bus: Arc<dyn Bus>) -> Result<(), BusError> {
    BUS_REGISTRY.lock().register(bus)
}

pub fn unregister_bus(name: &str) -> Result<(), BusError> {
    BUS_REGISTRY.lock().unregister(name)
}

pub fn list_buses() -> Vec<String> {
    BUS_REGISTRY.lock().list()
}

pub fn list_bus_devices(name: &str) -> Option<Vec<BusDeviceInfo>> {
    BUS_REGISTRY.lock().list_devices(name)
}
