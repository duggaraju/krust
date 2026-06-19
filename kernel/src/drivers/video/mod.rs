extern crate alloc;

use alloc::sync::Arc;
use bootloader_api::info::FrameBufferInfo;
use bootloader_x86_64_common::framebuffer::FrameBufferWriter;
use core::fmt::Write;
use log::info;
use spin::{Mutex, Once};
use vga::colors::{Color16, TextModeColor};
use vga::writers::{Graphics640x480x16, GraphicsWriter};
use vga::writers::{PrimitiveDrawing, ScreenCharacter, Text80x25, TextWriter};

use crate::drivers::traits::{Device, DeviceError, DeviceType};
use crate::module::traits::{KernelModule, KernelRegistry, ModuleError};

static FRAMEBUFFER: Once<Mutex<FrameBufferWriter>> = Once::new();

fn framebuffer() -> Option<&'static Mutex<FrameBufferWriter>> {
    FRAMEBUFFER.get()
}

/// Initialize the framebuffer-backed shell console.
pub fn init_framebuffer_console(framebuffer: &'static mut [u8], info: FrameBufferInfo) {
    let _ = FRAMEBUFFER.call_once(|| {
        info!("framebuffer console initialized");
        Mutex::new(FrameBufferWriter::new(framebuffer, info))
    });
}

/// Write shell output to the framebuffer console, if available.
pub fn write_str(s: &str) {
    if let Some(writer) = framebuffer() {
        let mut writer = writer.lock();
        let _ = writer.write_str(s);
    }
}

/// Clear the framebuffer (used when switching virtual consoles).
pub fn clear() {
    if let Some(writer) = framebuffer() {
        let mut writer = writer.lock();
        writer.clear();
    }
}

pub fn render_text(s: &str) {
    if let Some(writer) = framebuffer() {
        let mut writer = writer.lock();
        writer.clear();
        let _ = writer.write_str(s);
    }
}

pub fn is_initialized() -> bool {
    FRAMEBUFFER.get().is_some()
}

pub struct FrameBufferDevice;

pub struct FrameBufferDeviceModule;

impl FrameBufferDevice {
    pub fn new() -> Self {
        Self
    }
}

impl Device for FrameBufferDevice {
    fn name(&self) -> &str {
        "fb"
    }

    fn device_type(&self) -> DeviceType {
        DeviceType::Char
    }

    fn read(&self, _offset: usize, _buf: &mut [u8]) -> Result<usize, DeviceError> {
        Err(DeviceError::NotSupported)
    }

    fn write(&self, _offset: usize, buf: &[u8]) -> Result<usize, DeviceError> {
        let text = core::str::from_utf8(buf).map_err(|_| DeviceError::InvalidArgument)?;
        if !is_initialized() {
            return Err(DeviceError::NotReady);
        }
        write_str(text);
        Ok(buf.len())
    }

    fn seek(&self, _current: usize, _pos: crate::fs::vfs::SeekFrom) -> Result<usize, DeviceError> {
        Err(DeviceError::NotSupported)
    }
}

impl KernelModule for FrameBufferDeviceModule {
    fn name(&self) -> &str {
        "fbdev"
    }

    fn version(&self) -> &str {
        "0.1.0"
    }

    fn description(&self) -> &str {
        "Framebuffer device"
    }

    fn init(&self, registry: &dyn KernelRegistry) -> Result<(), ModuleError> {
        registry.register_device("fb", Arc::new(FrameBufferDevice::new()), 29, 0)?;
        Ok(())
    }

    fn cleanup(&self, registry: &dyn KernelRegistry) -> Result<(), ModuleError> {
        registry.unregister_device("fb")?;
        Ok(())
    }
}

pub fn print_something() {
    let text_mode = Text80x25::new();
    let color = TextModeColor::new(Color16::Yellow, Color16::Black);
    let screen_character = ScreenCharacter::new(b'T', color);

    info!("Printing something on the screen...");
    text_mode.set_mode();
    text_mode.clear_screen();
    text_mode.write_character(0, 0, screen_character);
}

pub fn print_graphics() {
    let mode = Graphics640x480x16::new();
    mode.set_mode();
    mode.clear_screen(Color16::Black);
    mode.draw_line((80, 60), (80, 420), Color16::White);
    mode.draw_line((80, 60), (540, 60), Color16::White);
    mode.draw_line((80, 420), (540, 420), Color16::White);
    mode.draw_line((540, 420), (540, 60), Color16::White);
    mode.draw_line((80, 90), (540, 90), Color16::White);
    for (offset, character) in "Hello World!".chars().enumerate() {
        mode.draw_character(270 + offset * 8, 72, character, Color16::White)
    }
}
