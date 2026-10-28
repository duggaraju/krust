extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use spin::Mutex;
use uart_16550::backend::PioBackend;
use uart_16550::{Config, Uart16550};

use crate::drivers::char::ldisc::{LineDiscipline, Termios2};
use crate::drivers::traits::{Device, DeviceError, DeviceType};

/// Standard x86 COM port addresses.
const COM_PORTS: [u16; 2] = [0x3F8, 0x2F8];

pub struct SerialPortDevice {
    name: String,
    uart: Mutex<Uart16550<PioBackend>>,
    ldisc: Mutex<LineDiscipline>,
}

impl SerialPortDevice {
    fn new(name: String, uart: Uart16550<PioBackend>) -> Self {
        Self {
            name,
            uart: Mutex::new(uart),
            ldisc: Mutex::new(LineDiscipline::new()),
        }
    }

    fn write_raw(&self, bytes: &[u8]) {
        self.uart.lock().send_bytes_exact(bytes);
    }

    fn read_cooked(
        &self,
        discipline: &Mutex<LineDiscipline>,
        buf: &mut [u8],
    ) -> Result<usize, DeviceError> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            let mut raw = [0; 64];
            let received = self.read(0, &mut raw)?;
            let mut output = Vec::new();
            let mut signal = None;
            let count = {
                let mut ldisc = discipline.lock();
                for &byte in &raw[..received] {
                    let echo = ldisc.process_input(byte);
                    if let Some(value) = echo.signal() {
                        signal = Some(value.to_kernel_signal());
                    }
                    echo.emit(|bytes| ldisc.process_output(bytes, &mut output));
                }
                ldisc.read(buf)
            };
            self.write_raw(&output);
            if let Some(signal) = signal {
                let _ = crate::process::with_current_task_mut(|task| {
                    task.deliver_signal(signal);
                });
            }
            if count != 0 {
                return Ok(count);
            }
            crate::process::idle_until_interrupt();
        }
    }
}

impl Device for SerialPortDevice {
    fn name(&self) -> &str {
        &self.name
    }

    fn device_type(&self) -> DeviceType {
        DeviceType::Char
    }

    /// Non-blocking drain: reads all bytes currently available in the UART
    /// receive FIFO into `buf`. Returns the number of bytes read.
    fn read(&self, _offset: usize, buf: &mut [u8]) -> Result<usize, DeviceError> {
        let mut uart = self.uart.lock();
        let mut count = 0;
        while count < buf.len() {
            match uart.try_receive_byte() {
                Ok(byte) => {
                    buf[count] = byte;
                    count += 1;
                }
                Err(_) => break,
            }
        }
        Ok(count)
    }

    fn read_blocking(&self, _offset: usize, buf: &mut [u8]) -> Result<usize, DeviceError> {
        self.read_cooked(&self.ldisc, buf)
    }

    fn terminal_settings(&self) -> Result<Termios2, DeviceError> {
        Ok(self.ldisc.lock().terminal_settings())
    }

    fn set_terminal_settings(&self, settings: Termios2, flush: bool) -> Result<(), DeviceError> {
        if flush {
            let mut raw = [0; 64];
            while self.read(0, &mut raw)? != 0 {}
        }
        self.ldisc.lock().set_terminal_settings(settings, flush);
        Ok(())
    }

    fn write(&self, _offset: usize, buf: &[u8]) -> Result<usize, DeviceError> {
        let mut output = Vec::new();
        self.ldisc.lock().process_output(buf, &mut output);
        self.write_raw(&output);
        Ok(buf.len())
    }
}

/// Serial console with line discipline and signal handling.
/// Wraps a SerialPortDevice with input processing (canonical mode, Ctrl-C → SIGTERM).
pub struct SerialConsoleDevice {
    name: String,
    device: SerialPortDevice,
    ldisc: Mutex<LineDiscipline>,
}

impl SerialConsoleDevice {
    fn new(device: SerialPortDevice) -> Self {
        let name = device.name.to_string();
        Self {
            name,
            device,
            ldisc: Mutex::new(LineDiscipline::new()),
        }
    }
}

impl Device for SerialConsoleDevice {
    fn name(&self) -> &str {
        &self.name
    }

    fn device_type(&self) -> DeviceType {
        DeviceType::Char
    }

    /// Read from the serial console with line discipline processing.
    /// Processes raw bytes through the line discipline (canonical mode, echo, signals).
    fn read(&self, _offset: usize, buf: &mut [u8]) -> Result<usize, DeviceError> {
        self.device.read_cooked(&self.ldisc, buf)
    }

    fn terminal_settings(&self) -> Result<Termios2, DeviceError> {
        Ok(self.ldisc.lock().terminal_settings())
    }

    fn set_terminal_settings(&self, settings: Termios2, flush: bool) -> Result<(), DeviceError> {
        self.ldisc.lock().set_terminal_settings(settings, flush);
        Ok(())
    }

    /// Write bytes to the serial console.
    fn write(&self, _offset: usize, buf: &[u8]) -> Result<usize, DeviceError> {
        let mut output = Vec::new();
        self.ldisc.lock().process_output(buf, &mut output);
        self.device.write_raw(&output);
        Ok(buf.len())
    }
}

/// Create and initialise a `SerialConsoleDevice` for the given COM port number
/// (1-based: `1` → COM1 / `ttyS0`, `2` → COM2 / `ttyS1`).
/// Returns `None` if the port number is out of range or UART init fails.
pub fn make_console(port_num: u8) -> Option<SerialConsoleDevice> {
    make(port_num).map(SerialConsoleDevice::new)
}

/// Create and initialise a `SerialPortDevice` for the given COM port number
/// (1-based: `1` → COM1 / `ttyS0`, `2` → COM2 / `ttyS1`).
/// Returns `None` if the port number is out of range or UART init fails.
pub fn make(port_num: u8) -> Option<SerialPortDevice> {
    let index = (port_num as usize).checked_sub(1)?;
    let addr = *COM_PORTS.get(index)?;
    // SAFETY: COM1/COM2 are standard x86 I/O port addresses.
    let mut uart = unsafe { Uart16550::new_port(addr) }.ok()?;
    uart.init(Config::default()).ok()?;
    let name = alloc::format!("ttyS{}", index);
    Some(SerialPortDevice::new(name, uart))
}

/// Convenience: initialise `ttyS0` (COM1). Kept for call sites that only need
/// one serial device.
pub fn init() -> SerialPortDevice {
    make(1).expect("failed to initialise ttyS0 (COM1)")
}
