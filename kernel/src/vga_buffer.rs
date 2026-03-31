
use log::info;
use vga::colors::{Color16, TextModeColor};
use vga::writers::{Graphics640x480x16, GraphicsWriter};
use vga::writers::{PrimitiveDrawing, ScreenCharacter, Text80x25, TextWriter};

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