extern crate alloc;

use alloc::vec::Vec;

use pc_keyboard::{
    DecodedKey, HandleControl, KeyCode, KeyState, PS2Keyboard, ScancodeSet1, layouts,
};
use spin::Mutex;
use x86_64::instructions::port::Port;

const PS2_STATUS_PORT: u16 = 0x64;
const PS2_DATA_PORT: u16 = 0x60;

pub struct Ps2Input {
    keyboard: Mutex<PS2Keyboard<layouts::Us104Key, ScancodeSet1>>,
    ctrl_down: Mutex<bool>,
    alt_down: Mutex<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ps2Event {
    InputByte(u8),
    SwitchVirtualConsole(usize),
}

impl Ps2Input {
    pub fn new() -> Self {
        Self {
            keyboard: Mutex::new(PS2Keyboard::new(
                ScancodeSet1::new(),
                layouts::Us104Key,
                HandleControl::MapLettersToUnicode,
            )),
            ctrl_down: Mutex::new(false),
            alt_down: Mutex::new(false),
        }
    }

    pub fn poll(&self, out: &mut Vec<Ps2Event>) {
        let mut status: Port<u8> = Port::new(PS2_STATUS_PORT);
        let mut data: Port<u8> = Port::new(PS2_DATA_PORT);

        while unsafe { status.read() } & 0x01 != 0 {
            let scancode = unsafe { data.read() };
            let mut keyboard = self.keyboard.lock();

            if let Ok(Some(key_event)) = keyboard.add_byte(scancode) {
                let code = key_event.code;
                let state = key_event.state;

                if matches!(code, KeyCode::LControl | KeyCode::RControl) {
                    *self.ctrl_down.lock() = is_key_press(state);
                }
                if matches!(code, KeyCode::LAlt | KeyCode::RAltGr) {
                    *self.alt_down.lock() = is_key_press(state);
                }

                if *self.ctrl_down.lock() && *self.alt_down.lock() {
                    if let Some(index) = function_key_to_vt(code, state) {
                        out.push(Ps2Event::SwitchVirtualConsole(index));
                        continue;
                    }
                }

                if let Some(decoded) = keyboard.process_keyevent(key_event) {
                    match decoded {
                        DecodedKey::Unicode(ch) if ch.is_ascii() => {
                            out.push(Ps2Event::InputByte(ch as u8));
                        }
                        DecodedKey::RawKey(KeyCode::Return)
                        | DecodedKey::RawKey(KeyCode::NumpadEnter) => {
                            out.push(Ps2Event::InputByte(b'\n'));
                        }
                        DecodedKey::RawKey(KeyCode::Backspace) => {
                            out.push(Ps2Event::InputByte(0x08));
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

fn function_key_to_vt(code: KeyCode, state: KeyState) -> Option<usize> {
    if !is_key_press(state) {
        return None;
    }

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