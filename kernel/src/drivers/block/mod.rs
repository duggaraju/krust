extern crate alloc;

pub mod sata;

use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::Mutex;

use crate::drivers::traits::{BlockDevice, DeviceError};

pub use sata::{SataBlockDevice, SataControllerLocation};

pub struct MemoryBlockDevice {
    sector_size: usize,
    sectors: u64,
    data: Mutex<Vec<u8>>,
}

impl MemoryBlockDevice {
    pub fn new(data: Vec<u8>, sector_size: usize) -> Result<Self, DeviceError> {
        if sector_size == 0 || data.len() % sector_size != 0 {
            return Err(DeviceError::InvalidArgument);
        }

        Ok(Self {
            sector_size,
            sectors: (data.len() / sector_size) as u64,
            data: Mutex::new(data),
        })
    }

    pub fn from_slice(data: &[u8], sector_size: usize) -> Result<Self, DeviceError> {
        Self::new(data.to_vec(), sector_size)
    }
}

impl BlockDevice for MemoryBlockDevice {
    fn sector_size(&self) -> usize {
        self.sector_size
    }

    fn sector_count(&self) -> u64 {
        self.sectors
    }

    fn read_sector(&self, sector: u64, buf: &mut [u8]) -> Result<(), DeviceError> {
        if sector >= self.sectors || buf.len() != self.sector_size {
            return Err(DeviceError::InvalidArgument);
        }

        let data = self.data.lock();
        let start = sector as usize * self.sector_size;
        buf.copy_from_slice(&data[start..start + self.sector_size]);
        Ok(())
    }

    fn write_sector(&self, sector: u64, buf: &[u8]) -> Result<(), DeviceError> {
        if sector >= self.sectors || buf.len() != self.sector_size {
            return Err(DeviceError::InvalidArgument);
        }

        let mut data = self.data.lock();
        let start = sector as usize * self.sector_size;
        data[start..start + self.sector_size].copy_from_slice(buf);
        Ok(())
    }
}

pub fn new_shared(data: Vec<u8>, sector_size: usize) -> Result<Arc<dyn BlockDevice>, DeviceError> {
    Ok(Arc::new(MemoryBlockDevice::new(data, sector_size)?))
}
