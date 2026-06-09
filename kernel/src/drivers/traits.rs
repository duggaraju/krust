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
    NotSupported,
}

pub trait Device: Send + Sync {
    fn name(&self) -> &str;
    fn device_type(&self) -> DeviceType;
}

pub trait CharDevice: Device {
    fn read(&self, buf: &mut [u8]) -> Result<usize, DeviceError>;
    fn write(&self, buf: &[u8]) -> Result<usize, DeviceError>;
}

pub trait BlockDevice: Device {
    fn read_block(&self, block: u64, buf: &mut [u8]) -> Result<(), DeviceError>;
    fn write_block(&self, block: u64, buf: &[u8]) -> Result<(), DeviceError>;
    fn block_size(&self) -> usize;
}
