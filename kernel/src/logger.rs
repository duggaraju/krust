#[cfg(feature = "arch-x86_64")]
use bootloader_api::info::FrameBufferInfo;
#[cfg(feature = "arch-x86_64")]
use bootloader_x86_64_common::framebuffer::FrameBufferWriter;
use core::fmt::Write;
use log::LevelFilter;
use spin::{Mutex, Once};
#[cfg(feature = "arch-x86_64")]
use uart_16550::backend::PioBackend;
#[cfg(feature = "arch-x86_64")]
use uart_16550::{Config, Uart16550};

#[cfg(feature = "arch-x86_64")]
struct KernelLogger {
    /// Framebuffer writer used only during early boot, before the tty manager is up.
    framebuffer: Option<Mutex<FrameBufferWriter>>,
    serial: Option<Mutex<Uart16550<PioBackend>>>,
}

#[cfg(not(feature = "arch-x86_64"))]
struct KernelLogger;

#[cfg(feature = "arch-x86_64")]
struct UartWriter<'a>(&'a mut Uart16550<PioBackend>);

#[cfg(feature = "arch-x86_64")]
impl core::fmt::Write for UartWriter<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.0.send_bytes_exact(s.as_bytes());
        Ok(())
    }
}

struct FmtBuf {
    buf: [u8; 512],
    len: usize,
}

impl FmtBuf {
    fn new() -> Self {
        Self {
            buf: [0u8; 512],
            len: 0,
        }
    }
    fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

impl core::fmt::Write for FmtBuf {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        let space = self.buf.len() - self.len;
        let n = bytes.len().min(space);
        self.buf[self.len..self.len + n].copy_from_slice(&bytes[..n]);
        self.len += n;
        Ok(())
    }
}

#[cfg(feature = "arch-x86_64")]
static LOGGER: Once<KernelLogger> = Once::new();

#[cfg(not(feature = "arch-x86_64"))]
static LOGGER: Once<KernelLogger> = Once::new();

#[cfg(feature = "arch-x86_64")]
pub fn init(
    framebuffer: Option<(&'static mut [u8], FrameBufferInfo)>,
    serial_port: Option<u16>,
    level: LevelFilter,
) {
    let logger = LOGGER.call_once(|| {
        let framebuffer =
            framebuffer.map(|(buffer, info)| Mutex::new(FrameBufferWriter::new(buffer, info)));
        let serial = serial_port.map(|port| {
            // SAFETY: the caller selects a standard x86 I/O port for serial output.
            let mut uart = unsafe { Uart16550::new_port(port) }
                .expect("invalid serial port address for logger");
            uart.init(Config::default())
                .expect("failed to initialize logger serial port");
            Mutex::new(uart)
        });
        KernelLogger {
            framebuffer,
            serial,
        }
    });

    log::set_logger(logger).expect("logger already set");
    log::set_max_level(level);
}

#[cfg(not(feature = "arch-x86_64"))]
pub fn init(
    framebuffer: Option<(&'static mut [u8], ())>,
    serial_port: Option<u16>,
    level: LevelFilter,
) {
    let _ = (framebuffer, serial_port);
    log::set_max_level(level);
}

#[cfg(feature = "arch-x86_64")]
impl log::Log for KernelLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        // Format once into a stack buffer
        let mut buf = FmtBuf::new();
        let _ = writeln!(buf, "{:5}: {}", record.level(), record.args());
        let bytes = buf.as_bytes();

        // Always write to serial
        if let Some(serial) = &self.serial {
            let mut serial = serial.lock();
            let _ = UartWriter(&mut serial)
                .write_str(core::str::from_utf8(bytes).unwrap_or("(invalid utf8)\n"));
        }

        let _ = &self.framebuffer;
    }

    fn flush(&self) {}
}
