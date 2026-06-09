extern crate alloc;

mod commands;
mod console;

use log::info;

use self::console::Console;
pub use self::console::ConsoleTarget;
const PROMPT: &str = "krust> ";

/// Entry point for the kernel shell task.
/// This runs as a kernel-mode loop reading cooked lines from the tty.
pub fn run(target: ConsoleTarget) -> ! {
    info!("kernel shell started");
    info!("shell: waiting for cooked tty line input");
    let mut console = Console::new(target);

    console.write_str("\nkrust kernel shell\nType 'help' for available commands.\n\n");
    console.write_str(PROMPT);

    loop {
        let line = console.read_line();
        info!("shell: received line len={} value={:?}", line.len(), line);
        if !line.is_empty() {
            info!("shell: dispatching command {:?}", line);
            commands::dispatch(&line, &mut console);
            info!("shell: command completed");
        }
        console.write_str(PROMPT);
    }
}

pub fn init() {
    crate::tty::init();
    info!("shell subsystem initialized");
}
