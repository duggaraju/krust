extern crate alloc;

use alloc::string::{String, ToString};
use spin::Mutex;
use uart_16550::backend::PioBackend;
use uart_16550::{Config, Uart16550};

use crate::drivers::char::ldisc::LineDiscipline;
use crate::drivers::traits::{Device, DeviceError, DeviceType};
use crate::process::task::Signal;

/// Standard x86 COM port addresses.
const COM_PORTS: [u16; 2] = [0x3F8, 0x2F8];

pub struct SerialPortDevice {
    name: String,
    uart: Mutex<Uart16550<PioBackend>>,
}

impl SerialPortDevice {
    fn new(name: String, uart: Uart16550<PioBackend>) -> Self {
        Self {
            name,
            uart: Mutex::new(uart),
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

    /// Write bytes to the serial port, translating bare `\n` to `\r\n`.
    fn write(&self, _offset: usize, buf: &[u8]) -> Result<usize, DeviceError> {
        let mut uart = self.uart.lock();
        for &byte in buf {
            match byte {
                b'\n' => uart.send_bytes_exact(b"\r\n"),
                _ => uart.send_bytes_exact(&[byte]),
            }
        }
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

    /// Deliver a signal to the current task
    fn deliver_signal(signal: Signal) {
        let _ = crate::process::with_current_task_mut(|task| {
            task.deliver_signal(signal);
        });
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
        if buf.is_empty() {
            return Ok(0);
        }

        // Read raw bytes from the serial port
        let mut raw_buf = [0u8; 64];
        let n = self.device.read(0, &mut raw_buf)?;

        let mut ldisc = self.ldisc.lock();
        let mut signal_to_deliver: Option<Signal> = None;

        // Process each byte through the line discipline
        for &byte in &raw_buf[..n] {
            let echo = ldisc.process_input(byte);

            // Check if a signal was generated and queue it
            if let Some(ldisc_signal) = echo.signal() {
                signal_to_deliver = Some(ldisc_signal.to_kernel_signal());
            }

            // Echo the byte back to the console (optional, like a real terminal)
            echo.emit(|echo_bytes| {
                let _ = self.device.write(0, echo_bytes);
            });
        }

        // Deliver signal to current task if one was generated
        if let Some(signal) = signal_to_deliver {
            Self::deliver_signal(signal);
        }

        // Read cooked bytes from the line discipline
        let bytes_read = ldisc.read(buf);
        Ok(bytes_read)
    }

    /// Write bytes to the serial console.
    fn write(&self, _offset: usize, buf: &[u8]) -> Result<usize, DeviceError> {
        self.device.write(_offset, buf)
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
