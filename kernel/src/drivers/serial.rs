use spin::Mutex;

use super::traits::{CharDevice, Device, DeviceError, DeviceType};
use uart_16550::Uart16550;
use uart_16550::backend::PioBackend;

const COM1_PORT: u16 = 0x3F8;
const SERIAL_NAME: &str = "serial0";

pub struct SerialPort {
    name: &'static str,
    uart: Mutex<Uart16550<PioBackend>>,
}

impl SerialPort {
    fn new(uart: Uart16550<PioBackend>) -> Self {
        Self {
            name: SERIAL_NAME,
            uart: Mutex::new(uart),
        }
    }
}

impl Device for SerialPort {
    fn name(&self) -> &str {
        self.name
    }

    fn device_type(&self) -> DeviceType {
        DeviceType::Char
    }
}

impl CharDevice for SerialPort {
    fn read(&self, _buf: &mut [u8]) -> Result<usize, DeviceError> {
        Err(DeviceError::NotSupported)
    }

    fn write(&self, buf: &[u8]) -> Result<usize, DeviceError> {
        let mut uart = self.uart.lock();
        uart.send_bytes_exact(buf);
        Ok(buf.len())
    }
}

pub fn init() -> SerialPort {
    // SAFETY: COM1 (0x3F8) is the standard serial port address on x86.
    let uart = unsafe { Uart16550::new_port(COM1_PORT) }.expect("invalid COM1 port address");
    SerialPort::new(uart)
}
