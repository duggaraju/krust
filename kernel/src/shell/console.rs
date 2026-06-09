#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsoleTarget {
    Serial,
    Framebuffer,
}

/// Terminal console for the shell — provides read and write over the tty layer.
pub struct Console {
    target: ConsoleTarget,
}

impl Console {
    pub fn new(target: ConsoleTarget) -> Self {
        crate::tty::init();
        Self { target }
    }

    /// Read a full cooked line from the terminal.
    pub fn read_line(&mut self) -> alloc::string::String {
        let mut tty = crate::tty::Tty::new();
        let mut line = alloc::string::String::new();

        loop {
            match tty.try_read_char() {
                Some('\r') | Some('\n') => {
                    self.write_str("\n");
                    return line;
                }
                Some('\u{0008}') | Some('\u{007f}') => {
                    if !line.is_empty() {
                        line.pop();
                        self.write_str("\x08 \x08");
                    }
                }
                Some('\u{0003}') => {
                    line.clear();
                    self.write_str("^C\n");
                }
                Some(ch) if ch.is_ascii_graphic() || ch == ' ' => {
                    line.push(ch);
                    let mut buf = [0u8; 4];
                    self.write_str(ch.encode_utf8(&mut buf));
                }
                _ => {
                    core::hint::spin_loop();
                }
            }
        }
    }

    /// Write a string to the console.
    pub fn write_str(&mut self, s: &str) {
        match self.target {
            ConsoleTarget::Serial => crate::tty::Tty::new().write_str(s),
            ConsoleTarget::Framebuffer => crate::drivers::video::write_str(s),
        }
    }
}
