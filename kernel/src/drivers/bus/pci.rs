extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::drivers::traits::{Bus, BusDeviceInfo, BusError, BusType, DeviceType};
use crate::module::traits::{KernelModule, KernelRegistry, ModuleError};

mod enumeration;

pub struct PciBus;

pub struct PciBusModule;

impl PciBus {
    pub fn new() -> Self {
        Self
    }

    fn scan_devices(&self) -> Result<Vec<BusDeviceInfo>, BusError> {
        let mut devices = Vec::new();
        let mut sata_index = 0usize;
        const LOG_LIMIT: usize = 32;

        enumeration::enumerate(read_config_dword, |function| {
            let ahci = function.is_ahci();
            let name = if ahci {
                let name = sata_name(sata_index);
                sata_index += 1;
                name
            } else {
                format!(
                    "pci{:02x}:{:02x}.{}",
                    function.bus, function.device, function.function
                )
            };
            if devices.len() < LOG_LIMIT {
                log::info!(
                    "PCI {:02x}:{:02x}.{} vendor={:04x} device={:04x} class={:02x}:{:02x}:{:02x} header={:02x} ahci={}",
                    function.bus,
                    function.device,
                    function.function,
                    function.vendor_id,
                    function.device_id,
                    function.class_code,
                    function.subclass,
                    function.prog_if,
                    function.header_type,
                    ahci
                );
            }
            devices.push(BusDeviceInfo {
                name,
                bus_type: BusType::Pci,
                device_type_hint: ahci.then_some(DeviceType::Block),
                pci_bus: Some(function.bus),
                pci_device: Some(function.device),
                pci_function: Some(function.function),
                vendor_id: Some(function.vendor_id),
                device_id: Some(function.device_id),
                header_type: Some(function.header_type),
                class_code: Some(function.class_code),
                subclass: Some(function.subclass),
                prog_if: Some(function.prog_if),
                bar5: function.bar5.map(u64::from),
            });
        })?;
        log::info!(
            "PCI enumeration: {} functions, {} AHCI controllers ({} detail logs omitted)",
            devices.len(),
            sata_index,
            devices.len().saturating_sub(LOG_LIMIT)
        );

        Ok(devices)
    }
}

impl Default for PciBus {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus for PciBus {
    fn name(&self) -> &str {
        "pci0"
    }

    fn bus_type(&self) -> BusType {
        BusType::Pci
    }

    fn enumerate(&self) -> Result<Vec<BusDeviceInfo>, BusError> {
        self.scan_devices()
    }
}

impl KernelModule for PciBusModule {
    fn name(&self) -> &str {
        "pci-bus"
    }

    fn version(&self) -> &str {
        "0.1.0"
    }

    fn description(&self) -> &str {
        "PCI bus enumeration"
    }

    fn init(&self, registry: &dyn KernelRegistry) -> Result<(), ModuleError> {
        registry.register_bus(Arc::new(PciBus::new()))?;
        Ok(())
    }

    fn cleanup(&self, registry: &dyn KernelRegistry) -> Result<(), ModuleError> {
        registry.unregister_bus("pci0")?;
        Ok(())
    }
}

/// Read an aligned dword from legacy PCI configuration space (first 256 bytes).
#[cfg(target_arch = "x86_64")]
pub fn read_config_dword(bus: u8, device: u8, function: u8, offset: u8) -> Result<u32, BusError> {
    use x86_64::instructions::port::Port;

    if device >= 32 || function >= 8 || offset & 3 != 0 {
        return Err(BusError::EnumerationFailed);
    }
    let address = 0x8000_0000u32
        | (u32::from(bus) << 16)
        | (u32::from(device) << 11)
        | (u32::from(function) << 8)
        | (u32::from(offset) & 0xfc);

    // CF8/CFC is one shared address/data pair, including across CPUs.
    static CONFIG_LOCK: spin::Mutex<()> = spin::Mutex::new(());
    x86_64::instructions::interrupts::without_interrupts(|| {
        let _guard = CONFIG_LOCK.lock();
        unsafe {
            let mut config_address: Port<u32> = Port::new(0xcf8);
            let mut config_data: Port<u32> = Port::new(0xcfc);
            config_address.write(address);
            Ok(config_data.read())
        }
    })
}

#[cfg(not(target_arch = "x86_64"))]
pub fn read_config_dword(
    _bus: u8,
    _device: u8,
    _function: u8,
    _offset: u8,
) -> Result<u32, BusError> {
    Err(BusError::EnumerationFailed)
}

fn sata_name(index: usize) -> String {
    let mut n = index;
    let mut letters = Vec::new();

    loop {
        letters.push((b'a' + (n % 26) as u8) as char);
        if n < 26 {
            break;
        }
        n = (n / 26) - 1;
    }

    letters.reverse();
    format!("sd{}", letters.into_iter().collect::<String>())
}
