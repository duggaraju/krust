extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::Ordering;

use ansi_parser::{AnsiParser, Output};
use log::info;
use pc_keyboard::{
    DecodedKey, HandleControl, KeyCode, KeyState, PS2Keyboard, ScancodeSet1, layouts,
};
use spin::{Mutex, Once};
use x86_64::instructions::port::Port;

use crate::drivers::char::ldisc::LineDiscipline;
use crate::drivers::registry;
use crate::drivers::traits::{Device, DeviceError, DeviceType};
use crate::process::task::ControllingTerminal;

const PS2_STATUS_PORT: u16 = 0x64;
const PS2_DATA_PORT: u16 = 0x60;
const MAX_SCREEN_BYTES: usize = 16 * 1024;
const VISIBLE_LINES: usize = 25;

/// The last virtual console is reserved for kernel log output.
/// Shell VCs start at 0; log VC is always the highest-indexed one.
pub const LOG_VC_OFFSET_FROM_END: usize = 1;

static KEYBOARD: Once<Mutex<PS2Keyboard<layouts::Us104Key, ScancodeSet1>>> = Once::new();
pub static MANAGER: Once<Mutex<TtyManager>> = Once::new();
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
    pending_output: Vec<u8>,
}

impl VirtualConsole {
    fn new() -> Self {
        Self {
            ldisc: LineDiscipline::new(),
            screen: String::new(),
            pending_output: Vec::new(),
        }
    }

    fn push_output(&mut self, bytes: &[u8]) {
        // Run bytes through the output discipline (ONLCR etc.) and update
        // the screen buffer for framebuffer rendering.
        let mut processed = alloc::vec::Vec::new();
        self.ldisc.process_output(bytes, &mut processed);
        self.pending_output.extend_from_slice(&processed);
        self.consume_pending_output();
    }

    fn push_raw_output(&self, bytes: &[u8]) -> alloc::vec::Vec<u8> {
        let mut processed = alloc::vec::Vec::new();
        self.ldisc.process_output(bytes, &mut processed);
        processed
    }

    fn consume_pending_output(&mut self) {
        const CLEAR_SCREEN: &[u8] = b"\x1b[2J";
        const CURSOR_HOME: &[u8] = b"\x1b[H";
        const ERASE_LEFT: &[u8] = b"\x1b[D \x1b[D";

        let mut consumed = 0usize;
        while consumed < self.pending_output.len() {
            let remaining = &self.pending_output[consumed..];

            if remaining.starts_with(CLEAR_SCREEN) {
                self.screen.clear();
                consumed += CLEAR_SCREEN.len();
                continue;
            } else if CLEAR_SCREEN.starts_with(remaining) {
                break;
            }

            if remaining.starts_with(CURSOR_HOME) {
                consumed += CURSOR_HOME.len();
                continue;
            } else if CURSOR_HOME.starts_with(remaining) {
                break;
            }

            if remaining.starts_with(ERASE_LEFT) {
                let _ = self.screen.pop();
                consumed += ERASE_LEFT.len();
                continue;
            } else if ERASE_LEFT.starts_with(remaining) {
                break;
            }

            let parse_len = complete_terminal_prefix_len(remaining);
            if parse_len == 0 {
                break;
            }
            let parse_slice = &remaining[..parse_len];
            if let Ok(text) = core::str::from_utf8(parse_slice) {
                for block in text.ansi_parse() {
                    if let Output::TextBlock(text) = block {
                        for byte in text.bytes() {
                            match byte {
                                b'\n' => self.screen.push('\n'),
                                b'\r' => {}
                                0x08 | 0x7f => {
                                    let _ = self.screen.pop();
                                }
                                byte => self.screen.push(byte as char),
                            }
                        }
                    }
                }
                consumed += parse_len;
            } else {
                // Keep non-UTF8 output visible rather than stalling parsing.
                self.screen.push(remaining[0] as char);
                consumed += 1;
            }
        }

        if consumed > 0 {
            self.pending_output.drain(..consumed);
        }

        if self.screen.len() > MAX_SCREEN_BYTES {
            let trim = self.screen.len() - MAX_SCREEN_BYTES;
            self.screen.drain(..trim);
        }
    }

    fn redraw(&self) {
        if crate::drivers::video::is_initialized() {
            crate::drivers::video::render_text(&self.visible_text());
        }
    }

    fn visible_text(&self) -> String {
        let mut lines: Vec<&str> = self.screen.lines().collect();
        if lines.is_empty() {
            return String::new();
        }

        let start = lines.len().saturating_sub(VISIBLE_LINES);
        lines.drain(..start);
        lines.join("\n")
    }

    fn should_use_viewport(&self) -> bool {
        self.screen.lines().count() > VISIBLE_LINES
    }

    fn needs_full_redraw(processed: &[u8]) -> bool {
        processed
            .iter()
            .any(|byte| matches!(byte, 0x08 | 0x7f | 0x1b))
    }
}

fn complete_terminal_prefix_len(bytes: &[u8]) -> usize {
    let Some(esc_idx) = bytes.iter().rposition(|&byte| byte == 0x1b) else {
        return bytes.len();
    };

    if esc_idx + 1 >= bytes.len() {
        return esc_idx;
    }

    match bytes[esc_idx + 1] {
        b'[' => {
            for &byte in &bytes[esc_idx + 2..] {
                if (b'@'..=b'~').contains(&byte) {
                    return bytes.len();
                }
            }
            esc_idx
        }
        b']' | b'P' | b'^' | b'_' => {
            let mut i = esc_idx + 2;
            while i < bytes.len() {
                match bytes[i] {
                    0x07 => return bytes.len(),
                    0x1b if i + 1 < bytes.len() && bytes[i + 1] == b'\\' => return bytes.len(),
                    _ => {}
                }
                i += 1;
            }
            esc_idx
        }
        _ => bytes.len(),
    }
}

pub struct TtyManager {
    vcs: Vec<VirtualConsole>,
    active: usize,
    ctrl_down: bool,
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
            ctrl_down: false,
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
                if self.vcs[index].should_use_viewport()
                    || VirtualConsole::needs_full_redraw(&processed)
                {
                    self.vcs[index].redraw();
                } else if let Ok(text) = core::str::from_utf8(&processed) {
                    crate::drivers::video::write_str(text);
                } else {
                    self.vcs[index].redraw();
                }
            } else {
                self.vcs[index].redraw();
            }
        }
    }

    fn queue_input_for_active(&mut self, byte: u8) {
        let echo = self.vcs[self.active].ldisc.process_input(byte);
        let active = self.active;

        // Check if a signal was generated (e.g., Ctrl-C)
        if let Some(ldisc_signal) = echo.signal() {
            Self::deliver_signal(ldisc_signal.to_kernel_signal());
        }

        // Deliver echo to the active VC's own output sink.
        echo.emit(|bytes| {
            let processed = self.vcs[active].push_raw_output(bytes);
            self.vcs[active].push_output(bytes);
            if SERIAL_ECHO.load(Ordering::Relaxed) {
                write_serial_bytes(&processed);
            }
            if crate::drivers::video::is_initialized() {
                if self.vcs[active].should_use_viewport()
                    || VirtualConsole::needs_full_redraw(&processed)
                {
                    self.vcs[active].redraw();
                } else if let Ok(text) = core::str::from_utf8(&processed) {
                    crate::drivers::video::write_str(text);
                } else {
                    self.vcs[active].redraw();
                }
            } else {
                self.vcs[active].redraw();
            }
        });
    }

    fn deliver_signal(signal: crate::process::task::Signal) {
        let _ = crate::process::with_current_task_mut(|task| {
            task.deliver_signal(signal);
        });
    }

    fn set_alt_key(&mut self, down: bool) {
        self.alt_down = down;
    }

    fn set_ctrl_key(&mut self, down: bool) {
        self.ctrl_down = down;
    }

    fn switch_to(&mut self, index: usize) {
        if index >= self.vcs.len() || index == self.active {
            return;
        }
        self.active = index;
        self.vcs[index].redraw();
        let message = format!("[tty{}]\n", index);
        if SERIAL_ECHO.load(Ordering::Relaxed) {
            write_serial_bytes(message.as_bytes());
        }
    }

    fn maybe_switch_from_key(&mut self, code: KeyCode, state: KeyState) -> bool {
        if !is_key_press(state) || !self.ctrl_down || !self.alt_down {
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

pub(crate) fn poll_input() {
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
            if matches!(code, KeyCode::LControl | KeyCode::RControl) {
                mgr.set_ctrl_key(is_key_press(state));
            }
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
                    | DecodedKey::RawKey(KeyCode::NumpadEnter) => mgr.queue_input_for_active(b'\n'),
                    DecodedKey::RawKey(KeyCode::Backspace) => mgr.queue_input_for_active(0x08),
                    // Shell line editor history navigation (Ctrl-P / Ctrl-N).
                    DecodedKey::RawKey(KeyCode::ArrowUp) => mgr.queue_input_for_active(0x10),
                    DecodedKey::RawKey(KeyCode::ArrowDown) => mgr.queue_input_for_active(0x0e),
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
    loop {
        poll_input();
        let read = manager().lock().read(vc_index, buf);
        if read != 0 {
            return read;
        }
        core::hint::spin_loop();
    }
}

pub(crate) fn tty_write(vc_index: usize, bytes: &[u8]) {
    manager().lock().write(vc_index, bytes);
}

/// Return the total number of virtual consoles.
pub fn tty_vc_count() -> usize {
    MANAGER.get().map(|m| m.lock().vcs.len()).unwrap_or(1)
}

/// Write directly to the kernel log virtual console (always the last VC).
/// Safe to call from the logger — does not acquire any logger locks.
pub fn tty_write_log(bytes: &[u8]) {
    let Some(manager) = MANAGER.get() else { return };
    let mut mgr = manager.lock();
    let log_vc = mgr.vcs.len().saturating_sub(LOG_VC_OFFSET_FROM_END);
    let active = mgr.active;
    let processed = mgr.vcs[log_vc].push_raw_output(bytes);
    mgr.vcs[log_vc].push_output(bytes);
    // Only paint to framebuffer if the log VC is currently active
    if log_vc == active && crate::drivers::video::is_initialized() {
        if mgr.vcs[log_vc].should_use_viewport() || VirtualConsole::needs_full_redraw(&processed) {
            mgr.vcs[log_vc].redraw();
        } else if let Ok(text) = core::str::from_utf8(&processed) {
            crate::drivers::video::write_str(text);
        } else {
            mgr.vcs[log_vc].redraw();
        }
    }
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

    fn seek(&self, _current: usize, _pos: crate::fs::vfs::SeekFrom) -> Result<usize, DeviceError> {
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

    fn seek(&self, _current: usize, _pos: crate::fs::vfs::SeekFrom) -> Result<usize, DeviceError> {
        Err(DeviceError::NotSupported)
    }
}
