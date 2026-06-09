extern crate alloc;

use alloc::format;
use alloc::vec::Vec;

use super::console::Console;

/// Dispatch a command line to the appropriate handler.
pub fn dispatch(line: &str, console: &mut Console) {
    let parts: Vec<&str> = line.trim().splitn(2, ' ').collect();
    let cmd = parts[0];
    let args = parts.get(1).unwrap_or(&"");

    match cmd {
        "help" => cmd_help(console),
        "echo" => cmd_echo(args, console),
        "clear" => cmd_clear(console),
        "mem" => cmd_mem(console),
        "ps" => cmd_ps(console),
        "ls" => cmd_ls(args, console),
        "cat" => cmd_cat(args, console),
        "mkdir" => cmd_mkdir(args, console),
        "write" => cmd_write(args, console),
        "lsdev" => cmd_lsdev(console),
        "lsmod" => cmd_lsmod(console),
        "uptime" => cmd_uptime(console),
        "shutdown" | "exit" => cmd_shutdown(),
        _ => {
            console.write_str(&format!("unknown command: '{}'\n", cmd));
        }
    }
}

fn cmd_help(console: &mut Console) {
    console.write_str("Available commands:\n");
    console.write_str("  help          - show this help\n");
    console.write_str("  echo <text>   - print text\n");
    console.write_str("  clear         - clear screen\n");
    console.write_str("  mem           - show memory stats\n");
    console.write_str("  ps            - list processes\n");
    console.write_str("  ls [path]     - list directory\n");
    console.write_str("  cat <path>    - read file contents\n");
    console.write_str("  mkdir <path>  - create directory\n");
    console.write_str("  write <path> <data> - write to file\n");
    console.write_str("  lsdev         - list devices\n");
    console.write_str("  lsmod         - list loaded modules\n");
    console.write_str("  uptime        - show ticks since boot\n");
    console.write_str("  shutdown      - shut down the VM\n");
    console.write_str("  exit          - alias for shutdown\n");
}

fn cmd_echo(args: &str, console: &mut Console) {
    console.write_str(args);
    console.write_str("\n");
}

fn cmd_clear(console: &mut Console) {
    // ANSI escape: clear screen and move cursor home
    console.write_str("\x1b[2J\x1b[H");
}

// --- Memory subsystem commands ---

#[cfg(feature = "mm")]
fn cmd_mem(console: &mut Console) {
    use crate::mm::heap::{HEAP_SIZE, HEAP_START};
    console.write_str(&format!(
        "Kernel heap: start=0x{:x}, size={} KiB\n",
        HEAP_START,
        HEAP_SIZE / 1024
    ));
}

#[cfg(not(feature = "mm"))]
fn cmd_mem(console: &mut Console) {
    console.write_str("memory management not enabled (feature 'mm' disabled)\n");
}

// --- Process subsystem commands ---

#[cfg(feature = "process")]
fn cmd_ps(console: &mut Console) {
    use crate::process::scheduler::SCHEDULER;
    let scheduler = SCHEDULER.lock();
    match scheduler.as_ref() {
        Some(sched) => {
            console.write_str("PID  STATE    NAME\n");
            console.write_str("---  -----    ----\n");
            if let Some(task) = sched.current() {
                console.write_str(&format!(
                    "{:<4} {:?}  {}\n",
                    task.pid, task.state, task.name
                ));
            }
        }
        None => console.write_str("scheduler not initialized\n"),
    }
}

#[cfg(not(feature = "process"))]
fn cmd_ps(console: &mut Console) {
    console.write_str("process management not enabled (feature 'process' disabled)\n");
}

// --- Filesystem commands ---

#[cfg(feature = "fs")]
fn cmd_ls(args: &str, console: &mut Console) {
    use crate::fs::vfs::{self, FileType};

    let root = match vfs::root_inode() {
        Ok(r) => r,
        Err(_) => {
            console.write_str("no filesystem mounted\n");
            return;
        }
    };

    let inode = if args.is_empty() || args == "/" {
        root
    } else {
        let name = args.trim_start_matches('/');
        match root.lookup(name) {
            Ok(i) => i,
            Err(e) => {
                console.write_str(&format!("ls: {:?}\n", e));
                return;
            }
        }
    };

    if inode.file_type() != FileType::Directory {
        console.write_str(&format!("{} (file, {} bytes)\n", args, inode.size()));
        return;
    }

    match inode.readdir() {
        Ok(entries) if entries.is_empty() => {
            console.write_str("(empty directory)\n");
        }
        Ok(entries) => {
            for entry in &entries {
                let type_indicator = match entry.file_type {
                    FileType::Directory => "/",
                    FileType::Symlink => "@",
                    FileType::CharDevice => "%",
                    FileType::BlockDevice => "#",
                    _ => "",
                };
                console.write_str(&format!("  {}{}\n", entry.name, type_indicator));
            }
        }
        Err(e) => console.write_str(&format!("ls: {:?}\n", e)),
    }
}

#[cfg(not(feature = "fs"))]
fn cmd_ls(_args: &str, console: &mut Console) {
    console.write_str("filesystem not enabled (feature 'fs' disabled)\n");
}

#[cfg(feature = "fs")]
fn cmd_cat(args: &str, console: &mut Console) {
    use crate::fs::vfs::{self, FileType};

    if args.is_empty() {
        console.write_str("usage: cat <filename>\n");
        return;
    }

    let root = match vfs::root_inode() {
        Ok(r) => r,
        Err(_) => {
            console.write_str("no filesystem mounted\n");
            return;
        }
    };

    let name = args.trim_start_matches('/');
    let inode = match root.lookup(name) {
        Ok(i) => i,
        Err(e) => {
            console.write_str(&format!("cat: {}: {:?}\n", name, e));
            return;
        }
    };

    if inode.file_type() == FileType::Directory {
        console.write_str(&format!("cat: {}: Is a directory\n", name));
        return;
    }

    let mut buf = [0u8; 512];
    let mut offset = 0;
    loop {
        match inode.read(offset, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if let Ok(s) = core::str::from_utf8(&buf[..n]) {
                    console.write_str(s);
                } else {
                    console.write_str("<binary data>\n");
                    break;
                }
                offset += n;
            }
            Err(e) => {
                console.write_str(&format!("\ncat: read error: {:?}\n", e));
                break;
            }
        }
    }
    console.write_str("\n");
}

#[cfg(not(feature = "fs"))]
fn cmd_cat(_args: &str, console: &mut Console) {
    console.write_str("filesystem not enabled (feature 'fs' disabled)\n");
}

#[cfg(feature = "fs")]
fn cmd_mkdir(args: &str, console: &mut Console) {
    use crate::fs::vfs::{self, FileType};

    if args.is_empty() {
        console.write_str("usage: mkdir <dirname>\n");
        return;
    }

    let root = match vfs::root_inode() {
        Ok(r) => r,
        Err(_) => {
            console.write_str("no filesystem mounted\n");
            return;
        }
    };

    let name = args.trim_start_matches('/');
    match root.create(name, FileType::Directory) {
        Ok(_) => console.write_str(&format!("created directory: {}\n", name)),
        Err(e) => console.write_str(&format!("mkdir: {:?}\n", e)),
    }
}

#[cfg(not(feature = "fs"))]
fn cmd_mkdir(_args: &str, console: &mut Console) {
    console.write_str("filesystem not enabled (feature 'fs' disabled)\n");
}

#[cfg(feature = "fs")]
fn cmd_write(args: &str, console: &mut Console) {
    use crate::fs::vfs::{self, FileType};

    let parts: Vec<&str> = args.splitn(2, ' ').collect();
    if parts.len() < 2 {
        console.write_str("usage: write <filename> <data>\n");
        return;
    }
    let name = parts[0].trim_start_matches('/');
    let data = parts[1];

    let root = match vfs::root_inode() {
        Ok(r) => r,
        Err(_) => {
            console.write_str("no filesystem mounted\n");
            return;
        }
    };

    // Create file if it doesn't exist
    let inode = match root.lookup(name) {
        Ok(i) => i,
        Err(_) => match root.create(name, FileType::Regular) {
            Ok(i) => i,
            Err(e) => {
                console.write_str(&format!("write: create failed: {:?}\n", e));
                return;
            }
        },
    };

    match inode.write(0, data.as_bytes()) {
        Ok(n) => console.write_str(&format!("wrote {} bytes to {}\n", n, name)),
        Err(e) => console.write_str(&format!("write: {:?}\n", e)),
    }
}

#[cfg(not(feature = "fs"))]
fn cmd_write(_args: &str, console: &mut Console) {
    console.write_str("filesystem not enabled (feature 'fs' disabled)\n");
}

// --- Device driver commands ---

#[cfg(feature = "drivers")]
fn cmd_lsdev(console: &mut Console) {
    use crate::drivers::registry;
    let devices = registry::list();
    if devices.is_empty() {
        console.write_str("no devices registered\n");
    } else {
        console.write_str("Registered devices:\n");
        for name in &devices {
            console.write_str(&format!("  {}\n", name));
        }
    }
}

#[cfg(not(feature = "drivers"))]
fn cmd_lsdev(console: &mut Console) {
    console.write_str("drivers not enabled (feature 'drivers' disabled)\n");
}

// --- Module commands ---

#[cfg(feature = "modules")]
fn cmd_lsmod(console: &mut Console) {
    use crate::module::registry::MODULE_REGISTRY;
    let registry = MODULE_REGISTRY.lock();
    let modules = registry.loaded_modules();
    if modules.is_empty() {
        console.write_str("no modules loaded\n");
    } else {
        console.write_str("Loaded modules:\n");
        for name in &modules {
            console.write_str(&format!("  {}\n", name));
        }
    }
}

#[cfg(not(feature = "modules"))]
fn cmd_lsmod(console: &mut Console) {
    console.write_str("modules not enabled (feature 'modules' disabled)\n");
}

// --- Misc ---

fn cmd_uptime(console: &mut Console) {
    // Read TSC as a rough "ticks since boot" indicator
    let ticks = unsafe { core::arch::x86_64::_rdtsc() };
    console.write_str(&format!("TSC ticks since boot: {}\n", ticks));
}

fn cmd_shutdown() -> ! {
    #[cfg(feature = "syscall")]
    {
        use crate::syscall::dispatch::SyscallArgs;
        use crate::syscall::handlers::sys_shutdown;

        let args = SyscallArgs {
            number: crate::syscall::numbers::SYS_SHUTDOWN,
            arg0: 0,
            arg1: 0,
            arg2: 0,
            arg3: 0,
            arg4: 0,
            arg5: 0,
        };
        let _ = sys_shutdown(&args);
        loop {
            x86_64::instructions::hlt();
        }
    }

    #[cfg(not(feature = "syscall"))]
    {
        crate::exit_qemu(crate::QemuExitCode::Success)
    }
}
