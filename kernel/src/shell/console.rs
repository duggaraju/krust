extern crate alloc;

use alloc::borrow::ToOwned;
use alloc::string::String;
use alloc::vec::Vec;

/// Terminal console for the shell.
///
/// I/O uses the shell task's inherited stdin/stdout file descriptors.
pub struct Console {
    stdin_fd: usize,
    stdout_fd: usize,
    history: Vec<String>,
}

impl Console {
    pub fn new() -> Self {
        Self {
            stdin_fd: 0,
            stdout_fd: 1,
            history: Vec::new(),
        }
    }

    pub fn push_history(&mut self, command: &str) {
        const HISTORY_LIMIT: usize = 8;
        if command.is_empty() {
            return;
        }
        self.history.push(command.to_owned());
        if self.history.len() > HISTORY_LIMIT {
            let overflow = self.history.len() - HISTORY_LIMIT;
            self.history.drain(0..overflow);
        }
    }

    fn redraw_input_line(&mut self, prompt: &str, line: &str) {
        self.write_str("\r\x1b[2K");
        self.write_str(prompt);
        self.write_str(line);
    }

    fn history_previous(
        &mut self,
        prompt: &str,
        line: &mut String,
        draft: &mut String,
        cursor: &mut Option<usize>,
    ) {
        if self.history.is_empty() {
            return;
        }

        let next = match *cursor {
            Some(index) if index > 0 => index - 1,
            Some(index) => index,
            None => {
                *draft = line.clone();
                self.history.len() - 1
            }
        };

        *cursor = Some(next);
        *line = self.history[next].clone();
        self.redraw_input_line(prompt, line);
    }

    fn history_next(
        &mut self,
        prompt: &str,
        line: &mut String,
        draft: &String,
        cursor: &mut Option<usize>,
    ) {
        let Some(index) = *cursor else {
            return;
        };

        if index + 1 < self.history.len() {
            let next = index + 1;
            *cursor = Some(next);
            *line = self.history[next].clone();
        } else {
            *cursor = None;
            *line = draft.clone();
        }
        self.redraw_input_line(prompt, line);
    }

    /// Read a full cooked line from the terminal.
    pub fn read_line(&mut self, prompt: &str) -> String {
        let mut line = String::new();
        let mut history_draft = String::new();
        let mut history_cursor: Option<usize> = None;
        let mut buf = [0u8; 1];

        loop {
            match crate::syscall::impls::read(self.stdin_fd, &mut buf) {
                Ok(0) => core::hint::spin_loop(),
                Ok(_) => match buf[0] as char {
                    '\r' | '\n' => return line,
                    '\u{0008}' | '\u{007f}' => {
                        history_cursor = None;
                        if !line.is_empty() {
                            line.pop();
                        }
                    }
                    '\u{0010}' => {
                        self.history_previous(
                            prompt,
                            &mut line,
                            &mut history_draft,
                            &mut history_cursor,
                        );
                    }
                    '\u{000e}' => {
                        self.history_next(prompt, &mut line, &history_draft, &mut history_cursor);
                    }
                    '\u{0003}' => {
                        history_cursor = None;
                        line.clear();
                    }
                    ch if ch.is_ascii_graphic() || ch == ' ' => {
                        history_cursor = None;
                        line.push(ch);
                    }
                    _ => {}
                },
                Err(_) => core::hint::spin_loop(),
            }
        }
    }

    /// Write a string to the console.
    pub fn write_str(&mut self, s: &str) {
        let _ = crate::syscall::impls::write(self.stdout_fd, s.as_bytes());
    }

    /// Write raw bytes directly without requiring a UTF-8 conversion.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        let _ = crate::syscall::impls::write(self.stdout_fd, bytes);
    }
}
