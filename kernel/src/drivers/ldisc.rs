//! Line discipline — shared between VirtualConsole (tty.rs) and PTY subsidiary (pty.rs).
//!
//! Implements a subset of POSIX termios cooked/canonical mode:
//!
//! Input processing (`process_input`):
//!   - ICRNL:  map incoming `\r` → `\n`.
//!   - ICANON: accumulate bytes into a line buffer; flush to the read queue on `\n`.
//!   - ERASE:  backspace (0x08) / DEL (0x7f) removes the last character from the line buffer.
//!   - ECHO:   each accepted byte is echoed; ERASE emits `\x08 \x08`.
//!   - ISIG:   Ctrl-C (0x03) discards the current line buffer and echoes `^C\n`.
//!
//! Output processing (`process_output`):
//!   - ONLCR:  map `\n` → `\r\n` before sending downstream.
//!
//! When `canonical` mode is **off** every input byte is placed directly into
//! the read queue with no buffering (raw/non-canonical mode).

extern crate alloc;

use alloc::collections::VecDeque;
use alloc::vec::Vec;

/// Maximum bytes held in the canonical line buffer before a forced flush.
const CANON_BUF_MAX: usize = 4096;

/// A line discipline instance.  One is embedded in each VirtualConsole and in
/// the `PtyShared` struct so both tty and pty paths share identical semantics.
pub struct LineDiscipline {
    // ── flags ─────────────────────────────────────────────────────────────────
    /// Echo input back to the output side.
    pub echo: bool,
    /// Canonical (line-buffered) mode; when false → raw mode.
    pub canonical: bool,
    /// Output: translate `\n` → `\r\n`.
    pub onlcr: bool,
    /// Input: translate `\r` → `\n`.
    pub icrnl: bool,
    /// Input: Ctrl-C sends interrupt (discard line, echo `^C`).
    pub isig: bool,

    // ── buffers ───────────────────────────────────────────────────────────────
    /// Canonical line buffer — bytes accumulate here until a newline.
    canon_buf: Vec<u8>,
    /// Cooked bytes ready to be consumed by a `read` call.
    read_queue: VecDeque<u8>,
}

impl LineDiscipline {
    /// Create a discipline with standard defaults (canonical, echo, ONLCR, ICRNL).
    pub fn new() -> Self {
        Self {
            echo: true,
            canonical: true,
            onlcr: true,
            icrnl: true,
            isig: true,
            canon_buf: Vec::new(),
            read_queue: VecDeque::new(),
        }
    }

    // ── Input path ────────────────────────────────────────────────────────────

    /// Feed one raw byte from the hardware/manager side into the discipline.
    ///
    /// Returns bytes that should be echoed back toward the output side
    /// (e.g. the screen or the PTY manager's read channel).  The caller is
    /// responsible for delivering any echo bytes to the appropriate sink.
    pub fn process_input(&mut self, byte: u8) -> EchoBytes {
        let byte = if self.icrnl && byte == b'\r' { b'\n' } else { byte };

        if !self.canonical {
            self.read_queue.push_back(byte);
            if self.echo {
                return EchoBytes::One(byte);
            }
            return EchoBytes::None;
        }

        // ── Canonical mode ────────────────────────────────────────────────────
        match byte {
            // ERASE — backspace / DEL
            0x08 | 0x7f => {
                if self.canon_buf.pop().is_some() && self.echo {
                    return EchoBytes::Erase; // "\x08 \x08"
                }
                return EchoBytes::None;
            }

            // INTR — Ctrl-C
            0x03 if self.isig => {
                self.canon_buf.clear();
                if self.echo {
                    return EchoBytes::CtrlC; // "^C\n"
                }
                return EchoBytes::None;
            }

            // KILL — Ctrl-U: erase entire line
            0x15 => {
                let had = !self.canon_buf.is_empty();
                self.canon_buf.clear();
                if had && self.echo {
                    return EchoBytes::KillLine;
                }
                return EchoBytes::None;
            }

            // Newline / line feed — flush line buffer
            b'\n' => {
                self.canon_buf.push(b'\n');
                for &b in &self.canon_buf {
                    self.read_queue.push_back(b);
                }
                self.canon_buf.clear();
                if self.echo {
                    return EchoBytes::One(b'\n');
                }
                return EchoBytes::None;
            }

            // Normal printable byte
            _ => {
                if self.canon_buf.len() < CANON_BUF_MAX {
                    self.canon_buf.push(byte);
                }
                if self.echo {
                    return EchoBytes::One(byte);
                }
                return EchoBytes::None;
            }
        }
    }

    /// How many bytes are ready to be read.
    pub fn read_ready(&self) -> usize {
        self.read_queue.len()
    }

    /// Drain up to `buf.len()` cooked bytes into `buf`.  Returns bytes copied.
    pub fn read(&mut self, buf: &mut [u8]) -> usize {
        let n = buf.len().min(self.read_queue.len());
        for i in 0..n {
            buf[i] = self.read_queue.pop_front().unwrap();
        }
        n
    }

    // ── Output path ───────────────────────────────────────────────────────────

    /// Process `input` bytes written by the application through the output
    /// discipline (e.g. ONLCR) and append them to `out`.
    pub fn process_output(&self, input: &[u8], out: &mut Vec<u8>) {
        for &byte in input {
            if self.onlcr && byte == b'\n' {
                out.push(b'\r');
            }
            out.push(byte);
        }
    }
}

impl Default for LineDiscipline {
    fn default() -> Self {
        Self::new()
    }
}

// ── Echo response ─────────────────────────────────────────────────────────────

/// What the caller should echo back after a single `process_input` call.
#[derive(Debug, Clone, Copy)]
pub enum EchoBytes {
    /// Nothing to echo.
    None,
    /// Echo exactly one byte.
    One(u8),
    /// Erase sequence: `\x08 \x08` (move back, overwrite with space, move back).
    Erase,
    /// Ctrl-C interrupt echo: `^C\n`.
    CtrlC,
    /// Ctrl-U kill-line echo: `^U\n`.
    KillLine,
}

impl EchoBytes {
    /// Write the echo sequence into `sink` using the provided write closure.
    pub fn emit<F: FnMut(&[u8])>(self, mut sink: F) {
        match self {
            EchoBytes::None => {}
            EchoBytes::One(b) => sink(&[b]),
            EchoBytes::Erase => sink(b"\x08 \x08"),
            EchoBytes::CtrlC => sink(b"^C\n"),
            EchoBytes::KillLine => sink(b"^U\n"),
        }
    }
}
