extern crate alloc;

mod commands;
mod console;

use self::console::Console;

const PROMPT: &str = "krust> ";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellExitReason {
    ExitRequested,
}

/// Entry point for the kernel shell task.
/// Reads its controlling terminal from the current process — set by the kernel
/// before calling this function.
pub fn run() -> ShellExitReason {
    let mut console = Console::new();

    console.write_str("\nkrust kernel shell\nType 'help' for available commands.\n\n");
    console.write_str(PROMPT);

    loop {
        let line = console.read_line(PROMPT);
        if !line.is_empty() {
            console.push_history(&line);
            let keep_running = commands::dispatch(&line, &mut console);
            if !keep_running {
                return ShellExitReason::ExitRequested;
            }
        }
        console.write_str(PROMPT);
    }
}
