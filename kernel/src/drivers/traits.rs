extern crate alloc;

use crate::fs::vfs::SeekFrom;
use alloc::vec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceType {
    Char,
    Block,
    Network,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceError {
    IoError,
    NotReady,
    InvalidArgument,
    Busy,
    NotFound,
    NotSupported,
}

pub trait Device: Send + Sync {
    fn name(&self) -> &str;
    fn device_type(&self) -> DeviceType;
    fn open(&self) -> Result<(), DeviceError> {
        Ok(())
    }
    fn close(&self) -> Result<(), DeviceError> {
        Ok(())
    }
    fn read(&self, _offset: usize, _buf: &mut [u8]) -> Result<usize, DeviceError> {
        Err(DeviceError::NotSupported)
    }
    fn write(&self, _offset: usize, _buf: &[u8]) -> Result<usize, DeviceError> {
        Err(DeviceError::NotSupported)
    }
    fn seek(&self, _current: usize, _pos: SeekFrom) -> Result<usize, DeviceError> {
        Err(DeviceError::NotSupported)
    }
    fn ioctl(&self, _request: usize, _arg: usize) -> Result<usize, DeviceError> {
        Err(DeviceError::NotSupported)
    }
}

pub trait BlockDevice: Send + Sync {
    fn sector_size(&self) -> usize;
    fn sector_count(&self) -> u64;
    fn read_sector(&self, sector: u64, buf: &mut [u8]) -> Result<(), DeviceError>;
    fn write_sector(&self, sector: u64, buf: &[u8]) -> Result<(), DeviceError>;
    fn sync(&self) -> Result<(), DeviceError> {
        Ok(())
    }

    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<(), DeviceError> {
        let sector_size = self.sector_size();
        if sector_size == 0 || buf.is_empty() {
            return Ok(());
        }

        let mut copied = 0usize;
        let mut current_offset = offset;
        let mut sector_buf = vec![0u8; sector_size];

        while copied < buf.len() {
            let sector = current_offset / sector_size;
            if sector as u64 >= self.sector_count() {
                return Err(DeviceError::InvalidArgument);
            }

            self.read_sector(sector as u64, &mut sector_buf)?;
            let sector_offset = current_offset % sector_size;
            let chunk = core::cmp::min(buf.len() - copied, sector_size - sector_offset);
            buf[copied..copied + chunk]
                .copy_from_slice(&sector_buf[sector_offset..sector_offset + chunk]);
            copied += chunk;
            current_offset += chunk;
        }

        Ok(())
    }

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<(), DeviceError> {
        let sector_size = self.sector_size();
        if sector_size == 0 || buf.is_empty() {
            return Ok(());
        }

        let mut written = 0usize;
        let mut current_offset = offset;
        let mut sector_buf = vec![0u8; sector_size];

        while written < buf.len() {
            let sector = current_offset / sector_size;
            if sector as u64 >= self.sector_count() {
                return Err(DeviceError::InvalidArgument);
            }

            let sector_offset = current_offset % sector_size;
            let chunk = core::cmp::min(buf.len() - written, sector_size - sector_offset);
            if chunk != sector_size {
                self.read_sector(sector as u64, &mut sector_buf)?;
            }
            sector_buf[sector_offset..sector_offset + chunk]
                .copy_from_slice(&buf[written..written + chunk]);
            self.write_sector(sector as u64, &sector_buf)?;
            written += chunk;
            current_offset += chunk;
        }

        Ok(())
    }
}
