extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::hint::spin_loop;
use spin::Mutex;

use crate::drivers::registry;
use crate::drivers::traits::{
    BlockDevice, BusDeviceInfo, BusType, Device, DeviceError, DeviceType, Driver, DriverError,
};
use crate::module::traits::{KernelModule, KernelRegistry, ModuleError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SataControllerLocation {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub bar5: u64,
}

pub struct SataBlockDevice {
    name: String,
    location: SataControllerLocation,
    state: Mutex<SataRuntime>,
}

impl SataBlockDevice {
    pub fn new(name: String, location: SataControllerLocation) -> Self {
        Self {
            name,
            location,
            state: Mutex::new(SataRuntime::default()),
        }
    }

    pub fn location(&self) -> SataControllerLocation {
        self.location
    }

    fn ensure_initialized(&self, state: &mut SataRuntime) -> Result<(), DeviceError> {
        if state.initialized {
            return Ok(());
        }

        let Some(abar_virt) = crate::arch::phys_to_virt_addr(self.location.bar5) else {
            return Err(DeviceError::NotReady);
        };

        let pi = mmio_read32(abar_virt, HBA_PI);
        let mut selected_port = None;
        let mut selected_sig = 0u32;

        for port in 0..32u32 {
            if (pi & (1u32 << port)) == 0 {
                continue;
            }

            let ssts = mmio_read32(abar_virt, HBA_PORT_BASE + port * HBA_PORT_STRIDE + PX_SSTS);
            let det = ssts & 0x0f;
            let ipm = (ssts >> 8) & 0x0f;
            if det != HBA_PORT_DET_PRESENT || ipm != HBA_PORT_IPM_ACTIVE {
                continue;
            }

            let sig = mmio_read32(abar_virt, HBA_PORT_BASE + port * HBA_PORT_STRIDE + PX_SIG);
            if sig == SATA_SIG_ATA {
                selected_port = Some(port as u8);
                selected_sig = sig;
                break;
            }
        }

        let Some(port) = selected_port else {
            return Err(DeviceError::NotFound);
        };

        if state.command_list.is_none() {
            state.command_list = Some(Box::new(AhciCommandList([0u8; 1024])));
        }
        if state.received_fis.is_none() {
            state.received_fis = Some(Box::new(AhciReceivedFis([0u8; 256])));
        }
        if state.command_table.is_none() {
            state.command_table = Some(Box::new(AhciCommandTable([0u8; 256])));
        }
        if state.dma_buffer.is_none() {
            state.dma_buffer = Some(Box::new(AhciDmaBuffer([0u8; 512])));
        }

        let command_list = state.command_list.as_mut().ok_or(DeviceError::NotReady)?;
        let received_fis = state.received_fis.as_mut().ok_or(DeviceError::NotReady)?;
        let command_table = state.command_table.as_mut().ok_or(DeviceError::NotReady)?;

        let command_list_phys = virt_to_phys(command_list.0.as_ptr() as u64)?;
        let fis_phys = virt_to_phys(received_fis.0.as_ptr() as u64)?;
        let command_table_phys = virt_to_phys(command_table.0.as_ptr() as u64)?;

        let base = HBA_PORT_BASE + u32::from(port) * HBA_PORT_STRIDE;
        ahci_stop_cmd(abar_virt, base)?;

        mmio_write32(abar_virt, base + PX_CLB, command_list_phys as u32);
        mmio_write32(abar_virt, base + PX_CLBU, (command_list_phys >> 32) as u32);
        mmio_write32(abar_virt, base + PX_FB, fis_phys as u32);
        mmio_write32(abar_virt, base + PX_FBU, (fis_phys >> 32) as u32);
        mmio_write32(abar_virt, base + PX_IS, u32::MAX);
        mmio_write32(abar_virt, base + PX_IE, 0);

        clear_ahci_buffers(command_list, received_fis, command_table);
        set_command_header(command_list, command_table_phys, false);
        ahci_start_cmd(abar_virt, base)?;

        let mut identify = [0u8; 512];
        let dma_buffer = state.dma_buffer.as_mut().ok_or(DeviceError::NotReady)?;
        let dma_phys = virt_to_phys(dma_buffer.0.as_ptr() as u64)?;
        ahci_rw(
            command_list,
            command_table,
            dma_buffer,
            abar_virt,
            base,
            false,
            ATA_CMD_IDENTIFY,
            0,
            1,
            dma_phys,
            512,
        )?;
        identify.copy_from_slice(&dma_buffer.0);

        let sectors_28 =
            u32::from_le_bytes([identify[120], identify[121], identify[122], identify[123]]) as u64;
        let sectors_48 = u64::from_le_bytes([
            identify[200],
            identify[201],
            identify[202],
            identify[203],
            identify[204],
            identify[205],
            identify[206],
            identify[207],
        ]);

        let use_lba48 = sectors_48 != 0;
        let sector_count = if use_lba48 { sectors_48 } else { sectors_28 };
        if sector_count == 0 {
            return Err(DeviceError::NotReady);
        }

        state.abar_virt = abar_virt;
        state.port = port;
        state.sector_count = sector_count;
        state.lba48 = use_lba48;
        state.signature = selected_sig;
        state.initialized = true;

        Ok(())
    }

    fn transfer_one_sector(
        &self,
        state: &mut SataRuntime,
        write: bool,
        sector: u64,
        buf: &mut [u8],
    ) -> Result<(), DeviceError> {
        self.ensure_initialized(state)?;

        if buf.len() != 512 || sector >= state.sector_count {
            return Err(DeviceError::InvalidArgument);
        }

        let base = HBA_PORT_BASE + u32::from(state.port) * HBA_PORT_STRIDE;

        let command_list = state.command_list.as_mut().ok_or(DeviceError::NotReady)?;
        let command_table = state.command_table.as_mut().ok_or(DeviceError::NotReady)?;
        let dma_buffer = state.dma_buffer.as_mut().ok_or(DeviceError::NotReady)?;
        let dma_phys = virt_to_phys(dma_buffer.0.as_ptr() as u64)?;
        if write {
            dma_buffer.0.copy_from_slice(buf);
        }

        let command = if write {
            if state.lba48 {
                ATA_CMD_WRITE_DMA_EXT
            } else {
                ATA_CMD_WRITE_DMA
            }
        } else if state.lba48 {
            ATA_CMD_READ_DMA_EXT
        } else {
            ATA_CMD_READ_DMA
        };

        ahci_rw(
            command_list,
            command_table,
            dma_buffer,
            state.abar_virt,
            base,
            write,
            command,
            sector,
            1,
            dma_phys,
            512,
        )?;

        if !write {
            buf.copy_from_slice(&dma_buffer.0);
        }

        Ok(())
    }
}

impl Device for SataBlockDevice {
    fn name(&self) -> &str {
        self.name.as_str()
    }

    fn device_type(&self) -> DeviceType {
        DeviceType::Block
    }

    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, DeviceError> {
        self.read_bytes(offset, buf)?;
        Ok(buf.len())
    }

    fn write(&self, offset: usize, buf: &[u8]) -> Result<usize, DeviceError> {
        self.write_bytes(offset, buf)?;
        Ok(buf.len())
    }
}

impl BlockDevice for SataBlockDevice {
    fn sector_size(&self) -> usize {
        512
    }

    fn sector_count(&self) -> u64 {
        let mut state = self.state.lock();
        if self.ensure_initialized(&mut state).is_err() {
            return 0;
        }
        state.sector_count
    }

    fn read_sector(&self, sector: u64, buf: &mut [u8]) -> Result<(), DeviceError> {
        let mut state = self.state.lock();
        self.transfer_one_sector(&mut state, false, sector, buf)
    }

    fn write_sector(&self, sector: u64, buf: &[u8]) -> Result<(), DeviceError> {
        let mut state = self.state.lock();
        let mut tmp = [0u8; 512];
        if buf.len() != tmp.len() {
            return Err(DeviceError::InvalidArgument);
        }
        tmp.copy_from_slice(buf);
        self.transfer_one_sector(&mut state, true, sector, &mut tmp)
    }
}

struct SataRuntime {
    initialized: bool,
    abar_virt: u64,
    port: u8,
    sector_count: u64,
    lba48: bool,
    signature: u32,
    command_list: Option<Box<AhciCommandList>>,
    received_fis: Option<Box<AhciReceivedFis>>,
    command_table: Option<Box<AhciCommandTable>>,
    dma_buffer: Option<Box<AhciDmaBuffer>>,
}

impl Default for SataRuntime {
    fn default() -> Self {
        Self {
            initialized: false,
            abar_virt: 0,
            port: 0,
            sector_count: 0,
            lba48: false,
            signature: 0,
            command_list: None,
            received_fis: None,
            command_table: None,
            dma_buffer: None,
        }
    }
}

const HBA_CAP: u32 = 0x00;
const HBA_PI: u32 = 0x0c;
const HBA_PORT_BASE: u32 = 0x100;
const HBA_PORT_STRIDE: u32 = 0x80;

const PX_CLB: u32 = 0x00;
const PX_CLBU: u32 = 0x04;
const PX_FB: u32 = 0x08;
const PX_FBU: u32 = 0x0c;
const PX_IS: u32 = 0x10;
const PX_IE: u32 = 0x14;
const PX_CMD: u32 = 0x18;
const PX_TFD: u32 = 0x20;
const PX_SIG: u32 = 0x24;
const PX_SSTS: u32 = 0x28;
const PX_SERR: u32 = 0x30;
const PX_SACT: u32 = 0x34;
const PX_CI: u32 = 0x38;

const HBA_PX_CMD_ST: u32 = 1 << 0;
const HBA_PX_CMD_FRE: u32 = 1 << 4;
const HBA_PX_CMD_FR: u32 = 1 << 14;
const HBA_PX_CMD_CR: u32 = 1 << 15;

const HBA_PX_IS_TFES: u32 = 1 << 30;
const HBA_PX_TFD_BSY: u32 = 1 << 7;
const HBA_PX_TFD_DRQ: u32 = 1 << 3;

const SATA_SIG_ATA: u32 = 0x0000_0101;
const HBA_PORT_DET_PRESENT: u32 = 0x03;
const HBA_PORT_IPM_ACTIVE: u32 = 0x01;

const FIS_TYPE_REG_H2D: u8 = 0x27;

const ATA_CMD_IDENTIFY: u8 = 0xec;
const ATA_CMD_READ_DMA: u8 = 0xc8;
const ATA_CMD_WRITE_DMA: u8 = 0xca;
const ATA_CMD_READ_DMA_EXT: u8 = 0x25;
const ATA_CMD_WRITE_DMA_EXT: u8 = 0x35;

const AHCI_TIMEOUT: usize = 1_000_000;

#[repr(align(1024))]
struct AhciCommandList([u8; 1024]);
#[repr(align(256))]
struct AhciReceivedFis([u8; 256]);
#[repr(align(128))]
struct AhciCommandTable([u8; 256]);
#[repr(align(512))]
struct AhciDmaBuffer([u8; 512]);

fn clear_ahci_buffers(
    command_list: &mut AhciCommandList,
    received_fis: &mut AhciReceivedFis,
    command_table: &mut AhciCommandTable,
) {
    command_list.0.fill(0);
    received_fis.0.fill(0);
    command_table.0.fill(0);
}

fn set_command_header(command_list: &mut AhciCommandList, command_table_phys: u64, write: bool) {
    let header = &mut command_list.0;
    let flags = (5u16 & 0x1f) | if write { 1 << 6 } else { 0 };
    header[0..2].copy_from_slice(&flags.to_le_bytes());
    header[2..4].copy_from_slice(&1u16.to_le_bytes());
    header[4..8].copy_from_slice(&0u32.to_le_bytes());
    header[8..12].copy_from_slice(&(command_table_phys as u32).to_le_bytes());
    header[12..16].copy_from_slice(&((command_table_phys >> 32) as u32).to_le_bytes());
}

fn build_command_fis(
    command_list: &mut AhciCommandList,
    command_table: &mut AhciCommandTable,
    dma_phys: u64,
    write: bool,
    command: u8,
    lba: u64,
    count: u16,
) {
    command_table.0.fill(0);
    let fis = &mut command_table.0[0..64];
    fis[0] = FIS_TYPE_REG_H2D;
    fis[1] = 1 << 7;
    fis[2] = command;
    fis[3] = 0;

    fis[4] = (lba & 0xff) as u8;
    fis[5] = ((lba >> 8) & 0xff) as u8;
    fis[6] = ((lba >> 16) & 0xff) as u8;
    fis[7] = 1 << 6;
    fis[8] = ((lba >> 24) & 0xff) as u8;
    fis[9] = ((lba >> 32) & 0xff) as u8;
    fis[10] = ((lba >> 40) & 0xff) as u8;
    fis[11] = 0;
    fis[12] = (count & 0xff) as u8;
    fis[13] = (count >> 8) as u8;
    fis[14] = 0;
    fis[15] = 0;

    let prdt = &mut command_table.0[0x80..0x90];
    prdt[0..4].copy_from_slice(&(dma_phys as u32).to_le_bytes());
    prdt[4..8].copy_from_slice(&((dma_phys >> 32) as u32).to_le_bytes());
    prdt[8..12].copy_from_slice(&0u32.to_le_bytes());
    let dbc_ioc = (512u32 - 1) | (1 << 31);
    prdt[12..16].copy_from_slice(&dbc_ioc.to_le_bytes());

    let command_table_phys = virt_to_phys(command_table.0.as_ptr() as u64).unwrap_or(0);
    set_command_header(command_list, command_table_phys, write);
}

fn ahci_rw(
    command_list: &mut AhciCommandList,
    command_table: &mut AhciCommandTable,
    dma_buffer: &mut AhciDmaBuffer,
    abar_virt: u64,
    port_base: u32,
    write: bool,
    command: u8,
    lba: u64,
    count: u16,
    dma_phys: u64,
    bytes: u32,
) -> Result<(), DeviceError> {
    let _ = bytes;
    let _ = dma_buffer;
    build_command_fis(
        command_list,
        command_table,
        dma_phys,
        write,
        command,
        lba,
        count,
    );

    if !wait_port_ready(abar_virt, port_base) {
        return Err(DeviceError::Busy);
    }

    mmio_write32(abar_virt, port_base + PX_IS, u32::MAX);
    mmio_write32(abar_virt, port_base + PX_SERR, u32::MAX);

    let slots = ((mmio_read32(abar_virt, HBA_CAP) >> 8) & 0x1f) + 1;
    let slot = find_free_slot(abar_virt, port_base, slots as usize).ok_or(DeviceError::Busy)?;

    let mut ci = mmio_read32(abar_virt, port_base + PX_CI);
    ci |= 1u32 << slot;
    mmio_write32(abar_virt, port_base + PX_CI, ci);

    let mut timeout = AHCI_TIMEOUT;
    while timeout > 0 {
        let ci_now = mmio_read32(abar_virt, port_base + PX_CI);
        if (ci_now & (1u32 << slot)) == 0 {
            break;
        }
        let is = mmio_read32(abar_virt, port_base + PX_IS);
        if (is & HBA_PX_IS_TFES) != 0 {
            return Err(DeviceError::IoError);
        }
        timeout -= 1;
        spin_loop();
    }

    if timeout == 0 {
        return Err(DeviceError::Busy);
    }

    let is = mmio_read32(abar_virt, port_base + PX_IS);
    if (is & HBA_PX_IS_TFES) != 0 {
        return Err(DeviceError::IoError);
    }

    Ok(())
}

fn find_free_slot(abar_virt: u64, port_base: u32, slots: usize) -> Option<u8> {
    let slots = slots.min(32);
    let occupied =
        mmio_read32(abar_virt, port_base + PX_SACT) | mmio_read32(abar_virt, port_base + PX_CI);
    (0..slots)
        .find(|idx| (occupied & (1u32 << idx)) == 0)
        .map(|idx| idx as u8)
}

fn wait_port_ready(abar_virt: u64, port_base: u32) -> bool {
    let mut timeout = AHCI_TIMEOUT;
    while timeout > 0 {
        let tfd = mmio_read32(abar_virt, port_base + PX_TFD);
        if (tfd & (HBA_PX_TFD_BSY | HBA_PX_TFD_DRQ)) == 0 {
            return true;
        }
        timeout -= 1;
        spin_loop();
    }
    false
}

fn ahci_stop_cmd(abar_virt: u64, port_base: u32) -> Result<(), DeviceError> {
    let mut cmd = mmio_read32(abar_virt, port_base + PX_CMD);
    cmd &= !HBA_PX_CMD_ST;
    cmd &= !HBA_PX_CMD_FRE;
    mmio_write32(abar_virt, port_base + PX_CMD, cmd);

    let mut timeout = AHCI_TIMEOUT;
    while timeout > 0 {
        let cmd_now = mmio_read32(abar_virt, port_base + PX_CMD);
        if (cmd_now & (HBA_PX_CMD_CR | HBA_PX_CMD_FR)) == 0 {
            return Ok(());
        }
        timeout -= 1;
        spin_loop();
    }

    Err(DeviceError::Busy)
}

fn ahci_start_cmd(abar_virt: u64, port_base: u32) -> Result<(), DeviceError> {
    let mut timeout = AHCI_TIMEOUT;
    while timeout > 0 {
        if (mmio_read32(abar_virt, port_base + PX_CMD) & HBA_PX_CMD_CR) == 0 {
            break;
        }
        timeout -= 1;
        spin_loop();
    }
    if timeout == 0 {
        return Err(DeviceError::Busy);
    }

    let mut cmd = mmio_read32(abar_virt, port_base + PX_CMD);
    cmd |= HBA_PX_CMD_FRE;
    cmd |= HBA_PX_CMD_ST;
    mmio_write32(abar_virt, port_base + PX_CMD, cmd);
    Ok(())
}

fn mmio_read32(base: u64, offset: u32) -> u32 {
    let addr = base.saturating_add(u64::from(offset)) as *const u32;
    unsafe { core::ptr::read_volatile(addr) }
}

fn mmio_write32(base: u64, offset: u32, value: u32) {
    let addr = base.saturating_add(u64::from(offset)) as *mut u32;
    unsafe {
        core::ptr::write_volatile(addr, value);
    }
}

fn virt_to_phys(virtual_addr: u64) -> Result<u64, DeviceError> {
    crate::arch::virt_to_phys_addr(virtual_addr).ok_or(DeviceError::NotReady)
}

// ─── Drive name generation ──────────────────────────────────────────────────

/// Returns a Linux-style `sd*` name for the n-th drive (0-indexed).
/// 0→"sda", 1→"sdb", …, 25→"sdz", 26→"sdaa", …
fn drive_letter_name(mut index: usize) -> String {
    let mut suffix: Vec<u8> = Vec::new();
    loop {
        suffix.push(b'a' + (index % 26) as u8);
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    suffix.reverse();
    let mut name = String::from("sd");
    for ch in suffix {
        name.push(ch as char);
    }
    name
}

// ─── MBR partition table ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
struct MbrPartition {
    part_type: u8,
    lba_start: u64,
    sector_count: u64,
}

/// Read sector 0 from `block`, validate the 0x55AA MBR signature, and return
/// up to 4 valid primary partition entries (type != 0, lba_start != 0, count != 0).
fn parse_mbr_partitions(block: &dyn BlockDevice) -> Vec<MbrPartition> {
    let mut mbr = [0u8; 512];
    if block.read_sector(0, &mut mbr).is_err() {
        return Vec::new();
    }
    if mbr[510] != 0x55 || mbr[511] != 0xAA {
        return Vec::new();
    }
    let mut parts = Vec::new();
    for i in 0..4usize {
        let off = 446 + i * 16;
        let part_type = mbr[off + 4];
        if part_type == 0x00 {
            continue;
        }
        let lba_start =
            u32::from_le_bytes([mbr[off + 8], mbr[off + 9], mbr[off + 10], mbr[off + 11]]) as u64;
        let sector_count =
            u32::from_le_bytes([mbr[off + 12], mbr[off + 13], mbr[off + 14], mbr[off + 15]]) as u64;
        if lba_start == 0 || sector_count == 0 {
            continue;
        }
        parts.push(MbrPartition {
            part_type,
            lba_start,
            sector_count,
        });
    }
    parts
}

// ─── Partition block device ───────────────────────────────────────────────────

/// A window into a contiguous range of sectors on an underlying `BlockDevice`.
/// Exposes each MBR partition as an independent, bounds-checked block device.
pub struct PartitionBlockDevice {
    name: String,
    inner: Arc<dyn BlockDevice>,
    lba_start: u64,
    sector_count: u64,
}

impl PartitionBlockDevice {
    fn new(name: String, inner: Arc<dyn BlockDevice>, lba_start: u64, sector_count: u64) -> Self {
        Self {
            name,
            inner,
            lba_start,
            sector_count,
        }
    }
}

impl Device for PartitionBlockDevice {
    fn name(&self) -> &str {
        &self.name
    }

    fn device_type(&self) -> DeviceType {
        DeviceType::Block
    }

    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, DeviceError> {
        self.read_bytes(offset, buf)?;
        Ok(buf.len())
    }

    fn write(&self, offset: usize, buf: &[u8]) -> Result<usize, DeviceError> {
        self.write_bytes(offset, buf)?;
        Ok(buf.len())
    }
}

impl BlockDevice for PartitionBlockDevice {
    fn sector_size(&self) -> usize {
        self.inner.sector_size()
    }

    fn sector_count(&self) -> u64 {
        self.sector_count
    }

    fn read_sector(&self, sector: u64, buf: &mut [u8]) -> Result<(), DeviceError> {
        if sector >= self.sector_count {
            return Err(DeviceError::InvalidArgument);
        }
        self.inner.read_sector(self.lba_start + sector, buf)
    }

    fn write_sector(&self, sector: u64, buf: &[u8]) -> Result<(), DeviceError> {
        if sector >= self.sector_count {
            return Err(DeviceError::InvalidArgument);
        }
        self.inner.write_sector(self.lba_start + sector, buf)
    }
}

pub struct SataDriver;

pub struct SataDeviceModule {
    registered_devices: Mutex<Vec<String>>,
    registered_fs_modules: Mutex<Vec<String>>,
}

impl SataDeviceModule {
    pub fn new() -> Self {
        Self {
            registered_devices: Mutex::new(Vec::new()),
            registered_fs_modules: Mutex::new(Vec::new()),
        }
    }

    fn discover_controllers(&self) -> Vec<BusDeviceInfo> {
        let mut controllers = Vec::new();
        for bus_name in registry::list_buses() {
            if let Some(devices) = registry::list_bus_devices(bus_name.as_str()) {
                for info in devices {
                    if info.bus_type == BusType::Pci
                        && info.class_code == Some(0x01)
                        && info.subclass == Some(0x06)
                        && info.prog_if == Some(0x01)
                    {
                        controllers.push(info);
                    }
                }
            }
        }
        controllers
    }

    fn register_controller_devices(
        &self,
        registry_api: &dyn KernelRegistry,
    ) -> Result<Vec<String>, ModuleError> {
        let controllers = self.discover_controllers();
        let mut names = Vec::new();
        let mut fs_modules = Vec::new();

        for (index, info) in controllers.into_iter().enumerate() {
            let Some(bus) = info.pci_bus else {
                continue;
            };
            let Some(device) = info.pci_device else {
                continue;
            };
            let Some(function) = info.pci_function else {
                continue;
            };
            let Some(bar5) = info.bar5 else {
                continue;
            };

            // Assign Linux-style sd* name: sda, sdb, …
            let drive_name = drive_letter_name(index);
            // Linux classic SCSI disk numbering: base minor = disk_index * 16.
            // partition minor = base + partition_number.
            let minor_base = (index as u16) * 16;

            let sata: Arc<SataBlockDevice> = Arc::new(SataBlockDevice::new(
                drive_name.clone(),
                SataControllerLocation {
                    bus,
                    device,
                    function,
                    bar5,
                },
            ));

            // Register the whole drive (e.g. sda, major 8 minor 0/16/32…).
            let dev: Arc<dyn Device> = Arc::clone(&sata) as Arc<dyn Device>;
            registry::register(&drive_name, dev, 8, minor_base)
                .map_err(|_| ModuleError::InitFailed)?;
            registry::register_block_device_alias(
                &drive_name,
                Arc::clone(&sata) as Arc<dyn BlockDevice>,
            )
            .map_err(|_| ModuleError::InitFailed)?;
            names.push(drive_name.clone());

            // Probe for MBR partitions.
            let sata_block: Arc<dyn BlockDevice> = sata;
            let partitions = parse_mbr_partitions(sata_block.as_ref());

            if partitions.is_empty() {
                // No partition table — treat whole drive as a mountable volume.
                log::info!(
                    "sata: {} has no MBR partition table, using whole drive",
                    drive_name
                );
                #[cfg(feature = "fs")]
                {
                    let module_name = alloc::format!("fatfs-{}", drive_name);
                    crate::fs::fat::register_module(
                        registry_api,
                        module_name.clone(),
                        Arc::clone(&sata_block),
                    );
                    fs_modules.push(module_name);
                }
            } else {
                // Register each partition as sda1, sda2, … and try to mount FAT on it.
                for (part_idx, partition) in partitions.into_iter().enumerate() {
                    let part_num = part_idx + 1; // 1-based
                    let part_name = alloc::format!("{}{}", drive_name, part_num);
                    let part_minor = minor_base + part_num as u16;

                    log::info!(
                        "sata: {}: partition {} type=0x{:02x} lba={}+{}",
                        drive_name,
                        part_num,
                        partition.part_type,
                        partition.lba_start,
                        partition.sector_count
                    );

                    let part_dev = Arc::new(PartitionBlockDevice::new(
                        part_name.clone(),
                        Arc::clone(&sata_block),
                        partition.lba_start,
                        partition.sector_count,
                    ));

                    let dev: Arc<dyn Device> = Arc::clone(&part_dev) as Arc<dyn Device>;
                    if registry::register(&part_name, dev, 8, part_minor).is_ok() {
                        let _ = registry::register_block_device_alias(
                            &part_name,
                            Arc::clone(&part_dev) as Arc<dyn BlockDevice>,
                        );
                        names.push(part_name.clone());
                        #[cfg(feature = "fs")]
                        {
                            let block: Arc<dyn BlockDevice> = part_dev;
                            let module_name = alloc::format!("fatfs-{}", part_name);
                            crate::fs::fat::register_module(
                                registry_api,
                                module_name.clone(),
                                block,
                            );
                            fs_modules.push(module_name);
                        }
                    } else {
                        log::warn!("sata: failed to register partition device {}", part_name);
                    }
                }
            }
        }

        *self.registered_fs_modules.lock() = fs_modules;

        Ok(names)
    }
}

impl Default for SataDeviceModule {
    fn default() -> Self {
        Self::new()
    }
}

impl Driver for SataDriver {
    fn name(&self) -> &str {
        "sata"
    }

    fn probe(&self, bus_device: &BusDeviceInfo) -> Result<Option<Arc<dyn Device>>, DriverError> {
        if bus_device.bus_type != BusType::Pci
            || bus_device.class_code != Some(0x01)
            || bus_device.subclass != Some(0x06)
            || bus_device.prog_if != Some(0x01)
        {
            return Ok(None);
        }

        let Some(bus) = bus_device.pci_bus else {
            return Err(DriverError::ProbeFailed);
        };
        let Some(device) = bus_device.pci_device else {
            return Err(DriverError::ProbeFailed);
        };
        let Some(function) = bus_device.pci_function else {
            return Err(DriverError::ProbeFailed);
        };
        let Some(bar5) = bus_device.bar5 else {
            return Err(DriverError::ProbeFailed);
        };

        let name = bus_device.name.clone();
        let location = SataControllerLocation {
            bus,
            device,
            function,
            bar5,
        };
        let sata = Arc::new(SataBlockDevice::new(name, location));
        Ok(Some(sata as Arc<dyn Device>))
    }
}

impl KernelModule for SataDeviceModule {
    fn name(&self) -> &str {
        "sata-dev"
    }

    fn version(&self) -> &str {
        "0.1.0"
    }

    fn description(&self) -> &str {
        "SATA block device registration"
    }

    fn init(&self, registry_api: &dyn KernelRegistry) -> Result<(), ModuleError> {
        let names = self.register_controller_devices(registry_api)?;
        *self.registered_devices.lock() = names;
        Ok(())
    }

    fn cleanup(&self, registry_api: &dyn KernelRegistry) -> Result<(), ModuleError> {
        for module_name in self.registered_fs_modules.lock().drain(..) {
            let _ = registry_api.unregister_module(&module_name);
        }
        for name in self.registered_devices.lock().drain(..) {
            let _ = registry::unregister(&name);
        }
        Ok(())
    }
}
