//! Line discipline — shared between VirtualConsole (tty.rs) and PTY subsidiary (pty.rs).
//!
//! Implements a subset of POSIX termios cooked/canonical mode:
//!
//! Input processing (`process_input`):
//!   - ICRNL:  map incoming `\r` → `\n`.
//!   - ICANON: accumulate bytes into a line buffer; flush to the read queue on `\n`.
//!   - ERASE:  backspace (0x08) / DEL (0x7f) removes the last character from the line buffer.
//!   - ECHO:   each accepted byte is echoed; ERASE emits `\x08 \x08`.
//!   - ISIG:   Ctrl-C (0x03) sends SIGTERM to the current task.
//!   - Ctrl-P/Ctrl-N pass through immediately (used for shell history up/down).
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

pub const TCGETS: usize = 0x5401;
pub const TCSETS: usize = 0x5402;
pub const TCSETSW: usize = 0x5403;
pub const TCSETSF: usize = 0x5404;
pub const TCGETS2: usize = 0x802C_542A;
pub const TCSETS2: usize = 0x402C_542B;
pub const TCSETSW2: usize = 0x402C_542C;
pub const TCSETSF2: usize = 0x402C_542D;

pub const ISIG: u32 = 1;
pub const ICANON: u32 = 2;
pub const ECHO: u32 = 8;
const ICRNL: u32 = 0x100;
const OPOST: u32 = 1;
const ONLCR: u32 = 4;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Termios {
    pub c_iflag: u32,
    pub c_oflag: u32,
    pub c_cflag: u32,
    pub c_lflag: u32,
    pub c_line: u8,
    pub c_cc: [u8; 19],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Termios2 {
    pub termios: Termios,
    pub c_ispeed: u32,
    pub c_ospeed: u32,
}

const _: () = assert!(core::mem::size_of::<Termios>() == 36);
const _: () = assert!(core::mem::size_of::<Termios2>() == 44);

impl Termios2 {
    pub fn as_bytes(&self) -> &[u8] {
        // Both ABI structures contain only initialized integers and no padding.
        unsafe {
            core::slice::from_raw_parts(
                core::ptr::from_ref(self).cast(),
                core::mem::size_of::<Self>(),
            )
        }
    }

    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        // Every bit pattern is valid for these integer-only ABI structures.
        unsafe {
            core::slice::from_raw_parts_mut(
                core::ptr::from_mut(self).cast(),
                core::mem::size_of::<Self>(),
            )
        }
    }
}

impl Default for Termios2 {
    fn default() -> Self {
        let mut c_cc = [0; 19];
        c_cc[..7].copy_from_slice(&[3, 28, 127, 21, 4, 0, 1]);
        c_cc[8] = 17;
        c_cc[9] = 19;
        c_cc[10] = 26;
        Self {
            termios: Termios {
                c_iflag: ICRNL,
                c_oflag: OPOST | ONLCR,
                c_cflag: 0x8bf, // B38400 | CS8 | CREAD | CLOCAL
                c_lflag: ISIG | ICANON | ECHO,
                c_line: 0,
                c_cc,
            },
            c_ispeed: 38400,
            c_ospeed: 38400,
        }
    }
}

/// A line discipline instance.  One is embedded in each VirtualConsole and in
/// the `PtyShared` struct so both tty and pty paths share identical semantics.
pub struct LineDiscipline {
    settings: Termios2,

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
            settings: Termios2::default(),
            canon_buf: Vec::new(),
            read_queue: VecDeque::new(),
        }
    }

    pub fn terminal_settings(&self) -> Termios2 {
        self.settings
    }

    pub fn set_terminal_settings(&mut self, settings: Termios2, flush: bool) {
        if flush {
            self.canon_buf.clear();
            self.read_queue.clear();
        } else if self.settings.termios.c_lflag & ICANON != 0
            && settings.termios.c_lflag & ICANON == 0
        {
            self.read_queue.extend(self.canon_buf.drain(..));
        }
        self.settings = settings;
    }

    // ── Input path ────────────────────────────────────────────────────────────

    /// Feed one raw byte from the hardware/manager side into the discipline.
    ///
    /// Returns bytes that should be echoed back toward the output side
    /// (e.g. the screen or the PTY manager's read channel).  The caller is
    /// responsible for delivering any echo bytes to the appropriate sink.
    pub fn process_input(&mut self, byte: u8) -> EchoBytes {
        let echo = self.settings.termios.c_lflag & ECHO != 0;
        let byte = if self.settings.termios.c_iflag & ICRNL != 0 && byte == b'\r' {
            b'\n'
        } else {
            byte
        };

        if self.settings.termios.c_lflag & ICANON == 0 {
            self.read_queue.push_back(byte);
            if echo {
                return EchoBytes::One(byte);
            }
            return EchoBytes::None;
        }

        // ── Canonical mode ────────────────────────────────────────────────────
        match byte {
            // Shell history navigation controls (Ctrl-P / Ctrl-N) are queued
            // immediately so interactive line editors can react before newline.
            0x10 | 0x0e => {
                self.read_queue.push_back(byte);
                return EchoBytes::None;
            }

            // ERASE — backspace / DEL
            0x08 | 0x7f => {
                if self.canon_buf.pop().is_some() && echo {
                    return EchoBytes::Erase; // "\x08 \x08"
                }
                return EchoBytes::None;
            }

            // INTR — Ctrl-C: send SIGTERM
            0x03 if self.settings.termios.c_lflag & ISIG != 0 => {
                self.canon_buf.clear();
                if echo {
                    return EchoBytes::CtrlC; // "^C\n"
                }
                return EchoBytes::None;
            }

            // KILL — Ctrl-U: erase entire line
            0x15 => {
                let had = !self.canon_buf.is_empty();
                self.canon_buf.clear();
                if had && echo {
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
                if echo {
                    return EchoBytes::One(b'\n');
                }
                return EchoBytes::None;
            }

            // Normal printable byte
            _ => {
                if self.canon_buf.len() < CANON_BUF_MAX {
                    self.canon_buf.push(byte);
                }
                if echo {
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
            if self.settings.termios.c_oflag & (OPOST | ONLCR) == (OPOST | ONLCR) && byte == b'\n' {
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

// ── Signal response ───────────────────────────────────────────────────────────

/// Signal detected during input processing.
/// Maps to kernel Signal enum via `to_kernel_signal()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// SIGTERM from Ctrl-C (user interrupt)
    Sigterm,
}

impl Signal {
    /// Convert to kernel signal type
    pub fn to_kernel_signal(&self) -> crate::process::task::Signal {
        match self {
            Signal::Sigterm => crate::process::task::Signal::Sigterm,
        }
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
            EchoBytes::Erase => sink(b"\x1b[D \x1b[D"),
            EchoBytes::CtrlC => sink(b"^C\n"),
            EchoBytes::KillLine => sink(b"^U\n"),
        }
    }

    /// Extract signal from echo bytes if one was generated.
    pub fn signal(&self) -> Option<Signal> {
        match self {
            EchoBytes::CtrlC => Some(Signal::Sigterm),
            _ => None,
        }
    }
}
