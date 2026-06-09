extern crate alloc;

/// Terminal console for the shell.
///
/// I/O is routed through `/dev/console` via the syscall layer.
pub struct Console {
    tty_fd: usize,
}

impl Console {
    pub fn new() -> Self {
        let tty_fd = crate::syscall::impls::open("/dev/console")
            .expect("failed to open /dev/console");
        Self { tty_fd }
    }

    /// Read a full cooked line from the terminal.
    pub fn read_line(&mut self) -> alloc::string::String {
        let mut line = alloc::string::String::new();
        let mut buf = [0u8; 1];

        loop {
            match crate::syscall::impls::read(self.tty_fd, &mut buf) {
                Ok(0) => core::hint::spin_loop(),
                Ok(_) => match buf[0] as char {
                    '\r' | '\n' => {
                        self.write_str("\n");
                        return line;
                    }
                    '\u{0008}' | '\u{007f}' => {
                        if !line.is_empty() {
                            line.pop();
                            self.write_str("\x08 \x08");
                        }
                    }
                    '\u{0003}' => {
                        line.clear();
                        self.write_str("^C\n");
                    }
                    ch if ch.is_ascii_graphic() || ch == ' ' => {
                        line.push(ch);
                        let mut ch_buf = [0u8; 4];
                        self.write_str(ch.encode_utf8(&mut ch_buf));
                    }
                    _ => {}
                },
                Err(_) => core::hint::spin_loop(),
            }
        }
    }

    /// Write a string to the console.
    pub fn write_str(&mut self, s: &str) {
        let _ = crate::syscall::impls::write(self.tty_fd, s.as_bytes());
    }

    /// Write raw bytes directly without requiring a UTF-8 conversion.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        let _ = crate::syscall::impls::write(self.tty_fd, bytes);
    }
}

impl Drop for Console {
    fn drop(&mut self) {
        let _ = crate::syscall::impls::close(self.tty_fd);
    }
}

