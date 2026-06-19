extern crate alloc;

mod commands;
mod console;

use log::debug;

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
    debug!("kernel shell started");
    let mut console = Console::new();

    console.write_str("\nkrust kernel shell\nType 'help' for available commands.\n\n");
    console.write_str(PROMPT);

    loop {
        let line = console.read_line(PROMPT);
        debug!("shell: received line len={} value={:?}", line.len(), line);
        if !line.is_empty() {
            console.push_history(&line);
            debug!("shell: dispatching command {:?}", line);
            let keep_running = commands::dispatch(&line, &mut console);
            debug!("shell: command completed");
            if !keep_running {
                debug!("shell: exit requested");
                return ShellExitReason::ExitRequested;
            }
        }
        console.write_str(PROMPT);
    }
}
