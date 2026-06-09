extern crate alloc;

use bootloader_api::info::FrameBufferInfo;
use bootloader_x86_64_common::framebuffer::FrameBufferWriter;
use core::fmt::Write;
use log::info;
use spin::{Mutex, Once};
use vga::colors::{Color16, TextModeColor};
use vga::writers::{Graphics640x480x16, GraphicsWriter};
use vga::writers::{PrimitiveDrawing, ScreenCharacter, Text80x25, TextWriter};

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
