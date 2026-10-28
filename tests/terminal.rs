#![allow(dead_code)]

mod process {
    pub mod task {
        pub enum Signal {
            Sigterm,
        }
    }
}

#[path = "../kernel/src/drivers/char/ldisc.rs"]
mod ldisc;

use ldisc::{ECHO, ICANON, LineDiscipline, Termios, Termios2};

fn input(discipline: &mut LineDiscipline, bytes: &[u8]) -> Vec<u8> {
    let mut echo = Vec::new();
    for &byte in bytes {
        discipline
            .process_input(byte)
            .emit(|bytes| echo.extend(bytes));
    }
    echo
}

#[test]
fn abi_layout_and_settings_round_trip() {
    assert_eq!(size_of::<Termios>(), 36);
    assert_eq!(size_of::<Termios2>(), 44);
    let original = Termios2::default();
    let mut copied = Termios2::default();
    copied.as_bytes_mut().copy_from_slice(original.as_bytes());
    assert_eq!(copied, original);
    assert_eq!(copied.termios.c_cc[6], 1);
}

#[test]
fn canonical_input_echoes_once_and_waits_for_newline() {
    let mut discipline = LineDiscipline::new();
    assert_eq!(input(&mut discipline, b"abc"), b"abc");
    let mut buffer = [0; 8];
    assert_eq!(discipline.read(&mut buffer), 0);
    assert_eq!(input(&mut discipline, b"\r"), b"\n");
    assert_eq!(discipline.read(&mut buffer), 4);
    assert_eq!(&buffer[..4], b"abc\n");
    assert_eq!(discipline.read(&mut buffer), 0);
}

#[test]
fn shell_raw_mode_disables_echo_and_line_buffering() {
    let mut discipline = LineDiscipline::new();
    let original = discipline.terminal_settings();
    let mut raw = original;
    raw.termios.c_lflag &= !(ECHO | ICANON);
    discipline.set_terminal_settings(raw, false);
    assert!(input(&mut discipline, b"a").is_empty());
    let mut buffer = [0; 8];
    assert_eq!(discipline.read(&mut buffer), 1);
    assert_eq!(buffer[0], b'a');
    discipline.set_terminal_settings(original, false);
    assert_eq!(input(&mut discipline, b"b"), b"b");
    assert_eq!(discipline.read(&mut buffer), 0);
}

#[test]
fn switching_to_raw_preserves_pending_input_but_flush_discards_it() {
    let mut discipline = LineDiscipline::new();
    input(&mut discipline, b"old");
    let mut raw = discipline.terminal_settings();
    raw.termios.c_lflag &= !(ECHO | ICANON);
    discipline.set_terminal_settings(raw, false);
    let mut buffer = [0; 8];
    assert_eq!(discipline.read(&mut buffer), 3);
    assert_eq!(&buffer[..3], b"old");
    input(&mut discipline, b"queued");
    discipline.set_terminal_settings(Termios2::default(), true);
    input(&mut discipline, b"partial");
    discipline.set_terminal_settings(raw, true);
    assert_eq!(discipline.read(&mut buffer), 0);
}

#[test]
fn output_processing_obeys_termios() {
    let mut discipline = LineDiscipline::new();
    let mut output = Vec::new();
    discipline.process_output(b"a\n", &mut output);
    assert_eq!(output, b"a\r\n");
    let mut settings = discipline.terminal_settings();
    settings.termios.c_oflag = 0;
    discipline.set_terminal_settings(settings, false);
    output.clear();
    discipline.process_output(b"b\n", &mut output);
    assert_eq!(output, b"b\n");
}
