extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::Ordering;

use log::info;
use pc_keyboard::{
    DecodedKey, HandleControl, KeyCode, KeyState, PS2Keyboard, ScancodeSet1, layouts,
};
use spin::{Mutex, Once};
use x86_64::instructions::port::Port;

use crate::drivers::registry;
use crate::drivers::traits::{Device, DeviceError, DeviceType};
use crate::drivers::ldisc::LineDiscipline;
use crate::process::task::ControllingTerminal;

const PS2_STATUS_PORT: u16 = 0x64;
const PS2_DATA_PORT: u16 = 0x60;
const MAX_SCREEN_BYTES: usize = 16 * 1024;

static KEYBOARD: Once<Mutex<PS2Keyboard<layouts::Us104Key, ScancodeSet1>>> = Once::new();
static MANAGER: Once<Mutex<TtyManager>> = Once::new();
static SERIAL_ECHO: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(true);
static SERIAL_DEVICE: Mutex<Option<Arc<dyn Device>>> = Mutex::new(None);

pub fn init_core(virtual_console_count: usize) {
    static INIT: Once<()> = Once::new();
    let _ = INIT.call_once(|| {
        let _ = keyboard();
        let _ = MANAGER.call_once(|| Mutex::new(TtyManager::new(virtual_console_count.max(1))));
        info!("tty: initialized with virtual consoles");
    });
}

pub fn set_serial_echo(enabled: bool) {
    SERIAL_ECHO.store(enabled, Ordering::Relaxed);
}

pub fn set_serial_device(device: Arc<dyn Device>) {
    *SERIAL_DEVICE.lock() = Some(device);
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

fn manager() -> &'static Mutex<TtyManager> {
    MANAGER.call_once(|| panic!("tty manager not initialized"))
}

struct VirtualConsole {
    ldisc: LineDiscipline,
    screen: String,
}

impl VirtualConsole {
    fn new() -> Self {
        Self {
            ldisc: LineDiscipline::new(),
            screen: String::new(),
        }
    }

    fn push_output(&mut self, bytes: &[u8]) {
        // Run bytes through the output discipline (ONLCR etc.) and update
        // the screen buffer for framebuffer rendering.
        let mut processed = alloc::vec::Vec::new();
        self.ldisc.process_output(bytes, &mut processed);
        for &byte in &processed {
            match byte {
                b'\r' => {}
                b'\n' => self.screen.push('\n'),
                0x08 | 0x7f => { let _ = self.screen.pop(); }
                _ => self.screen.push(byte as char),
            }
        }
        if self.screen.len() > MAX_SCREEN_BYTES {
            let trim = self.screen.len() - MAX_SCREEN_BYTES;
            self.screen.drain(..trim);
        }
    }

    fn push_raw_output(&self, bytes: &[u8]) -> alloc::vec::Vec<u8> {
        let mut processed = alloc::vec::Vec::new();
        self.ldisc.process_output(bytes, &mut processed);
        processed
    }
}

struct TtyManager {
    vcs: Vec<VirtualConsole>,
    active: usize,
    alt_down: bool,
}

impl TtyManager {
    fn new(count: usize) -> Self {
        let mut vcs = Vec::with_capacity(count);
        for _ in 0..count {
            vcs.push(VirtualConsole::new());
        }
        Self {
            vcs,
            active: 0,
            alt_down: false,
        }
    }

    fn resolve_index(&self, index: usize) -> usize {
        index.min(self.vcs.len().saturating_sub(1))
    }

    fn read(&mut self, index: usize, buf: &mut [u8]) -> usize {
        let index = self.resolve_index(index);
        self.vcs[index].ldisc.read(buf)
    }

    fn write(&mut self, index: usize, bytes: &[u8]) {
        let index = self.resolve_index(index);
        let processed = self.vcs[index].push_raw_output(bytes);
        self.vcs[index].push_output(bytes);

        if index == self.active {
            if SERIAL_ECHO.load(Ordering::Relaxed) {
                write_serial_bytes(&processed);
            }
            if crate::drivers::video::is_initialized() {
                if let Ok(text) = core::str::from_utf8(&processed) {
                    crate::drivers::video::write_str(text);
                }
            }
        }
    }

    fn queue_input_for_active(&mut self, byte: u8) {
        let echo = self.vcs[self.active].ldisc.process_input(byte);
        let active = self.active;
        // Deliver echo to the active VC's own output sink.
        echo.emit(|bytes| {
            let processed = self.vcs[active].push_raw_output(bytes);
            self.vcs[active].push_output(bytes);
            if SERIAL_ECHO.load(Ordering::Relaxed) {
                write_serial_bytes(&processed);
            }
            if crate::drivers::video::is_initialized() {
                if let Ok(text) = core::str::from_utf8(&processed) {
                    crate::drivers::video::write_str(text);
                }
            }
        });
    }

    fn set_alt_key(&mut self, down: bool) {
        self.alt_down = down;
    }

    fn switch_to(&mut self, index: usize) {
        if index >= self.vcs.len() || index == self.active {
            return;
        }
        self.active = index;
        let message = format!("\n[switched to tty{}]\n", index);
        if SERIAL_ECHO.load(Ordering::Relaxed) {
            write_serial_bytes(message.as_bytes());
        }
        if crate::drivers::video::is_initialized() {
            crate::drivers::video::write_str(&message);
            crate::drivers::video::write_str(&self.vcs[index].screen);
        }
    }

    fn maybe_switch_from_key(&mut self, code: KeyCode, state: KeyState) -> bool {
        if !is_key_press(state) || !self.alt_down {
            return false;
        }

        let Some(index) = function_key_to_vt(code) else {
            return false;
        };
        self.switch_to(index);
        true
    }
}

fn function_key_to_vt(code: KeyCode) -> Option<usize> {
    match code {
        KeyCode::F1 => Some(0),
        KeyCode::F2 => Some(1),
        KeyCode::F3 => Some(2),
        KeyCode::F4 => Some(3),
        KeyCode::F5 => Some(4),
        KeyCode::F6 => Some(5),
        KeyCode::F7 => Some(6),
        KeyCode::F8 => Some(7),
        KeyCode::F9 => Some(8),
        KeyCode::F10 => Some(9),
        KeyCode::F11 => Some(10),
        KeyCode::F12 => Some(11),
        _ => None,
    }
}

fn is_key_press(state: KeyState) -> bool {
    matches!(state, KeyState::Down | KeyState::SingleShot)
}

fn poll_input() {
    let serial = SERIAL_DEVICE.lock().clone();
    if let Some(device) = serial {
        let mut buf = [0u8; 64];
        if let Ok(n) = device.read(0, &mut buf) {
            if n > 0 {
                let mut mgr = manager().lock();
                for &byte in &buf[..n] {
                    mgr.queue_input_for_active(byte);
                }
            }
        }
    }

    let mut status: Port<u8> = Port::new(PS2_STATUS_PORT);
    let mut data: Port<u8> = Port::new(PS2_DATA_PORT);
    let mut mgr = manager().lock();
    while unsafe { status.read() } & 0x01 != 0 {
        let scancode = unsafe { data.read() };
        let mut keyboard = keyboard().lock();
        if let Ok(Some(key_event)) = keyboard.add_byte(scancode) {
            let code = key_event.code;
            let state = key_event.state;
            if matches!(code, KeyCode::LAlt | KeyCode::RAltGr) {
                mgr.set_alt_key(is_key_press(state));
            }
            if mgr.maybe_switch_from_key(code, state) {
                continue;
            }

            if let Some(decoded) = keyboard.process_keyevent(key_event) {
                match decoded {
                    DecodedKey::Unicode(ch) if ch.is_ascii() => {
                        mgr.queue_input_for_active(ch as u8);
                    }
                    DecodedKey::RawKey(KeyCode::Return)
                    | DecodedKey::RawKey(KeyCode::NumpadEnter) => {
                        mgr.queue_input_for_active(b'\n')
                    }
                    DecodedKey::RawKey(KeyCode::Backspace) => mgr.queue_input_for_active(0x08),
                    _ => {}
                }
            }
        }
    }
}

fn write_serial_bytes(bytes: &[u8]) {
    let device = SERIAL_DEVICE.lock().clone();
    if let Some(device) = device {
        let _ = device.write(0, bytes);
    }
}

pub(crate) fn tty_read(vc_index: usize, buf: &mut [u8]) -> usize {
    if buf.is_empty() {
        return 0;
    }
    poll_input();
    manager().lock().read(vc_index, buf)
}

pub(crate) fn tty_write(vc_index: usize, bytes: &[u8]) {
    manager().lock().write(vc_index, bytes);
}

/// A character device representing the current task's controlling terminal.
///
/// The actual backing target is stored in the task entry and resolved on each
/// I/O operation from the currently running process.
pub struct TtyDevice {
    name: String,
}

/// A character device representing one fixed virtual console (`tty0`..`ttyN`).
pub struct VirtualConsoleDevice {
    name: String,
    index: usize,
}

impl TtyDevice {
    pub fn new_console() -> Self {
        Self {
            name: String::from("console"),
        }
    }

    fn current_target(&self) -> Result<ControllingTerminal, DeviceError> {
        crate::process::current_controlling_terminal().ok_or(DeviceError::NotFound)
    }

    fn serial_device(index: usize) -> Result<alloc::sync::Arc<dyn Device>, DeviceError> {
        let name = format!("ttyS{}", index);
        registry::get(name.as_str()).ok_or(DeviceError::NotFound)
    }
}

impl VirtualConsoleDevice {
    pub fn new(index: usize) -> Self {
        Self {
            name: format!("tty{}", index),
            index,
        }
    }
}

impl Device for TtyDevice {
    fn name(&self) -> &str {
        &self.name
    }

    fn device_type(&self) -> DeviceType {
        DeviceType::Char
    }

    fn read(&self, _offset: usize, buf: &mut [u8]) -> Result<usize, DeviceError> {
        match self.current_target()? {
            ControllingTerminal::VirtualConsole(index) => Ok(tty_read(index, buf)),
            ControllingTerminal::Serial(index) => Self::serial_device(index)?.read(0, buf),
        }
    }

    fn write(&self, _offset: usize, buf: &[u8]) -> Result<usize, DeviceError> {
        match self.current_target()? {
            ControllingTerminal::VirtualConsole(index) => {
                tty_write(index, buf);
                Ok(buf.len())
            }
            ControllingTerminal::Serial(index) => Self::serial_device(index)?.write(0, buf),
        }
    }

    fn seek(
        &self,
        _current: usize,
        _pos: crate::fs::vfs::SeekFrom,
    ) -> Result<usize, DeviceError> {
        Err(DeviceError::NotSupported)
    }
}

impl Device for VirtualConsoleDevice {
    fn name(&self) -> &str {
        &self.name
    }

    fn device_type(&self) -> DeviceType {
        DeviceType::Char
    }

    fn read(&self, _offset: usize, buf: &mut [u8]) -> Result<usize, DeviceError> {
        Ok(tty_read(self.index, buf))
    }

    fn write(&self, _offset: usize, buf: &[u8]) -> Result<usize, DeviceError> {
        tty_write(self.index, buf);
        Ok(buf.len())
    }

    fn seek(
        &self,
        _current: usize,
        _pos: crate::fs::vfs::SeekFrom,
    ) -> Result<usize, DeviceError> {
        Err(DeviceError::NotSupported)
    }
}
