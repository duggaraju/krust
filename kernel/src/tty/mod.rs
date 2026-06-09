extern crate alloc;

use alloc::collections::VecDeque;
use core::sync::atomic::{AtomicU16, Ordering};

use log::info;
use pc_keyboard::{DecodedKey, HandleControl, KeyCode, PS2Keyboard, ScancodeSet1, layouts};
use spin::{Mutex, Once};
use uart_16550::backend::PioBackend;
use uart_16550::{Config, Uart16550};
use x86_64::instructions::port::Port;

const COM1_PORT: u16 = 0x3F8;
const COM2_PORT: u16 = 0x2F8;
const PS2_STATUS_PORT: u16 = 0x64;
const PS2_DATA_PORT: u16 = 0x60;
static UART: Once<Mutex<Uart16550<PioBackend>>> = Once::new();
static KEYBOARD: Once<Mutex<PS2Keyboard<layouts::Us104Key, ScancodeSet1>>> = Once::new();
static DISCIPLINE: Once<Mutex<LineDiscipline>> = Once::new();
static PORT: AtomicU16 = AtomicU16::new(COM1_PORT);

pub fn set_port(port: u8) {
    let addr = match port {
        1 => COM1_PORT,
        2 => COM2_PORT,
        _ => COM1_PORT,
    };
    PORT.store(addr, Ordering::Relaxed);
}

fn uart() -> &'static Mutex<Uart16550<PioBackend>> {
    UART.call_once(|| {
        let port = PORT.load(Ordering::Relaxed);
        // SAFETY: COM1/COM2 are the standard x86 serial ports.
        let mut uart = unsafe { Uart16550::new_port(port) }.expect("invalid serial port address");
        uart.init(Config::default())
            .expect("failed to initialize serial port");
        Mutex::new(uart)
    })
}

fn keyboard() -> &'static Mutex<PS2Keyboard<layouts::Us104Key, ScancodeSet1>> {
    KEYBOARD.call_once(|| {
        Mutex::new(PS2Keyboard::new(
            ScancodeSet1::new(),
            layouts::Us104Key,
            HandleControl::MapLettersToUnicode,
        ))
    })
}

fn discipline() -> &'static Mutex<LineDiscipline> {
    DISCIPLINE.call_once(|| Mutex::new(LineDiscipline::new()))
}

/// Initialize the terminal layer.
pub fn init() {
    static INIT: Once<()> = Once::new();
    let _ = INIT.call_once(|| {
        let _ = uart();
        let _ = keyboard();
        let _ = discipline();
        info!("tty: initialized serial/keyboard line discipline");
    });
}

struct LineDiscipline {
    pending_chars: VecDeque<char>,
}

impl LineDiscipline {
    fn new() -> Self {
        Self {
            pending_chars: VecDeque::new(),
        }
    }

    fn on_char(&mut self, ch: char) {
        info!("tty: rx char={:?}", ch);
        self.pending_chars.push_back(ch);
    }

    fn poll_serial(&mut self, uart: &mut Uart16550<PioBackend>) {
        while let Ok(byte) = uart.try_receive_byte() {
            self.on_char(byte as char);
        }
    }

    fn poll_keyboard(&mut self) {
        let mut status: Port<u8> = Port::new(PS2_STATUS_PORT);
        let mut data: Port<u8> = Port::new(PS2_DATA_PORT);

        while unsafe { status.read() } & 0x01 != 0 {
            let scancode = unsafe { data.read() };
            let mut keyboard = keyboard().lock();
            if let Ok(Some(key_event)) = keyboard.add_byte(scancode) {
                if let Some(decoded) = keyboard.process_keyevent(key_event) {
                    match decoded {
                        DecodedKey::Unicode(ch) => self.on_char(ch),
                        DecodedKey::RawKey(KeyCode::Return) => self.on_char('\n'),
                        DecodedKey::RawKey(KeyCode::NumpadEnter) => self.on_char('\n'),
                        DecodedKey::RawKey(KeyCode::Backspace) => self.on_char('\u{0008}'),
                        _ => {}
                    }
                }
            }
        }
    }

    fn pop_char(&mut self) -> Option<char> {
        self.pending_chars.pop_front()
    }
}

/// Read/write terminal access over the serial line.
pub struct Tty;

impl Tty {
    pub fn new() -> Self {
        init();
        Self
    }

    /// Returns the next raw input character, if one is already buffered.
    pub fn try_read_char(&mut self) -> Option<char> {
        let mut uart = uart().lock();
        let mut discipline = discipline().lock();
        discipline.poll_keyboard();
        discipline.poll_serial(&mut uart);
        discipline.pop_char()
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) {
        let mut uart = uart().lock();
        uart.send_bytes_exact(bytes);
    }

    pub fn write_str(&mut self, s: &str) {
        let mut uart = uart().lock();
        for byte in s.bytes() {
            match byte {
                b'\n' => uart.send_bytes_exact(b"\r\n"),
                _ => uart.send_bytes_exact(&[byte]),
            }
        }
    }
}
