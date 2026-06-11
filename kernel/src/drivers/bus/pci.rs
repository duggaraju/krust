extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::drivers::traits::{Bus, BusDeviceInfo, BusError, BusType, DeviceType};
use crate::module::traits::{KernelModule, KernelRegistry, ModuleError};

pub struct PciBus;

pub struct PciBusModule;

impl PciBus {
    pub fn new() -> Self {
        Self
    }

    fn scan_sata_devices(&self) -> Result<Vec<BusDeviceInfo>, BusError> {
        let mut devices = Vec::new();
        let mut index = 0usize;

        for bus in 0u8..=255 {
            for device in 0u8..32 {
                let header = read_header(bus, device, 0)?;
                if header.vendor_id == 0xffff {
                    continue;
                }

                let function_count = if header.multi_function { 8 } else { 1 };
                for function in 0u8..function_count {
                    let header = read_header(bus, device, function)?;
                    if header.vendor_id == 0xffff {
                        continue;
                    }

                    if header.class_code == 0x01
                        && header.subclass == 0x06
                        && header.prog_if == 0x01
                    {
                        let name = sata_name(index);
                        devices.push(BusDeviceInfo {
                            name,
                            bus_type: BusType::Pci,
                            device_type_hint: Some(DeviceType::Block),
                            pci_bus: Some(bus),
                            pci_device: Some(device),
                            pci_function: Some(function),
                            class_code: Some(header.class_code),
                            subclass: Some(header.subclass),
                            prog_if: Some(header.prog_if),
                            bar5: Some(u64::from(header.bar5 & 0xffff_fff0)),
                        });
                        index = index.saturating_add(1);
                    }
                }
            }
        }

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
        self.scan_sata_devices()
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

#[cfg(target_arch = "x86_64")]
fn read_header(bus: u8, device: u8, function: u8) -> Result<PciHeader, BusError> {
    let vendor_id = read_config_word(bus, device, function, 0x00)?;
    let header_type = read_config_byte(bus, device, function, 0x0e)?;
    let class_data = read_config_dword(bus, device, function, 0x08)?;
    let bar5 = read_config_dword(bus, device, function, 0x24)?;

    Ok(PciHeader {
        vendor_id,
        class_code: (class_data >> 24) as u8,
        subclass: (class_data >> 16) as u8,
        prog_if: (class_data >> 8) as u8,
        multi_function: header_type & 0x80 != 0,
        bar5,
    })
}

#[cfg(not(target_arch = "x86_64"))]
fn read_header(_bus: u8, _device: u8, _function: u8) -> Result<PciHeader, BusError> {
    Err(BusError::DiscoveryFailed)
}

#[cfg(target_arch = "x86_64")]
fn read_config_dword(bus: u8, device: u8, function: u8, offset: u8) -> Result<u32, BusError> {
    use x86_64::instructions::port::Port;

    let address = 0x8000_0000u32
        | (u32::from(bus) << 16)
        | (u32::from(device) << 11)
        | (u32::from(function) << 8)
        | (u32::from(offset) & 0xfc);

    unsafe {
        let mut config_address: Port<u32> = Port::new(0xcf8);
        let mut config_data: Port<u32> = Port::new(0xcfc);
        config_address.write(address);
        Ok(config_data.read())
    }
}

#[cfg(target_arch = "x86_64")]
fn read_config_word(bus: u8, device: u8, function: u8, offset: u8) -> Result<u16, BusError> {
    let dword = read_config_dword(bus, device, function, offset)?;
    let shift = u32::from(offset & 0x02) * 8;
    Ok(((dword >> shift) & 0xffff) as u16)
}

#[cfg(target_arch = "x86_64")]
fn read_config_byte(bus: u8, device: u8, function: u8, offset: u8) -> Result<u8, BusError> {
    let dword = read_config_dword(bus, device, function, offset)?;
    let shift = u32::from(offset & 0x03) * 8;
    Ok(((dword >> shift) & 0xff) as u8)
}

#[cfg(not(target_arch = "x86_64"))]
fn read_config_dword(_bus: u8, _device: u8, _function: u8, _offset: u8) -> Result<u32, BusError> {
    Err(BusError::DiscoveryFailed)
}

#[cfg(not(target_arch = "x86_64"))]
fn read_config_word(_bus: u8, _device: u8, _function: u8, _offset: u8) -> Result<u16, BusError> {
    Err(BusError::DiscoveryFailed)
}

#[cfg(not(target_arch = "x86_64"))]
fn read_config_byte(_bus: u8, _device: u8, _function: u8, _offset: u8) -> Result<u8, BusError> {
    Err(BusError::DiscoveryFailed)
}

struct PciHeader {
    vendor_id: u16,
    class_code: u8,
    subclass: u8,
    prog_if: u8,
    multi_function: bool,
    bar5: u32,
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