use bootloader_api::info::FrameBufferInfo;
use bootloader_x86_64_common::framebuffer::FrameBufferWriter;
use core::fmt::Write;
use log::LevelFilter;
use spin::{Mutex, Once};
use uart_16550::backend::PioBackend;
use uart_16550::{Config, Uart16550};

struct KernelLogger {
    framebuffer: Option<Mutex<FrameBufferWriter>>,
    serial: Option<Mutex<Uart16550<PioBackend>>>,
}

struct UartWriter<'a>(&'a mut Uart16550<PioBackend>);

impl core::fmt::Write for UartWriter<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.0.send_bytes_exact(s.as_bytes());
        Ok(())
    }
}

static LOGGER: Once<KernelLogger> = Once::new();

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

impl log::Log for KernelLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        if let Some(framebuffer) = &self.framebuffer {
            let mut framebuffer = framebuffer.lock();
            let _ = writeln!(framebuffer, "{:5}: {}", record.level(), record.args());
        }

        if let Some(serial) = &self.serial {
            let mut serial = serial.lock();
            let _ = writeln!(
                UartWriter(&mut serial),
                "{:5}: {}",
                record.level(),
                record.args()
            );
        }
    }

    fn flush(&self) {}
}
