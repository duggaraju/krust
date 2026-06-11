use crate::drivers::traits::{Device, DeviceError, DeviceType};

pub struct NullDevice;

impl NullDevice {
    pub fn new() -> Self {
        Self
    }
}

impl Device for NullDevice {
    fn name(&self) -> &str {
        "null"
    }

    fn device_type(&self) -> DeviceType {
        DeviceType::Char
    }

    fn read(&self, _offset: usize, buf: &mut [u8]) -> Result<usize, DeviceError> {
        buf.fill(0);
        Ok(buf.len())
    }

    fn write(&self, _offset: usize, buf: &[u8]) -> Result<usize, DeviceError> {
        Ok(buf.len())
    }
}
