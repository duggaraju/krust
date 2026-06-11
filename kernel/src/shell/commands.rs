extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::console::Console;

/// Dispatch a command line to the appropriate handler.
pub fn dispatch(line: &str, console: &mut Console) -> bool {
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
        "stat" => cmd_stat(args, console),
        "cat" => cmd_cat(args, console),
        "cd" => cmd_cd(args, console),
        "mkdir" => cmd_mkdir(args, console),
        "write" => cmd_write(args, console),
        "lsdev" => cmd_lsdev(console),
        "lspci" => cmd_lspci(console),
        "lsmod" => cmd_lsmod(console),
        "uptime" => cmd_uptime(console),
        "shutdown" | "exit" => return false,
        _ => {
            console.write_str(&format!("unknown command: '{}'\n", cmd));
        }
    }

    true
}

fn cmd_help(console: &mut Console) {
    console.write_str("Available commands:\n");
    console.write_str("  help          - show this help\n");
    console.write_str("  echo <text>   - print text\n");
    console.write_str("  clear         - clear screen\n");
    console.write_str("  mem           - show memory stats\n");
    console.write_str("  ps            - list processes\n");
    console.write_str("  ls [-a] [-l] [path] - list directory\n");
    console.write_str("  stat <path>   - show inode details\n");
    console.write_str("  cat <path>    - read file contents\n");
    console.write_str("  cd <path>     - change directory\n");
    console.write_str("  mkdir <path>  - create directory\n");
    console.write_str("  write <path> <data> - write to file\n");
    console.write_str("  lsdev         - list devices\n");
    console.write_str("  lspci         - list PCI bus devices\n");
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
    use crate::syscall::impls;

    match impls::open("/proc/meminfo") {
        Ok(fd) => {
            let mut buf = [0u8; 256];
            loop {
                match impls::read(fd, &mut buf) {
                    Ok(0) => break,
                    Ok(n) => console.write_bytes(&buf[..n]),
                    Err(_) => break,
                }
            }
            let _ = impls::close(fd);
        }
        Err(errno) => console.write_str(&format!("mem: errno {}\n", errno)),
    }
}

#[cfg(not(feature = "mm"))]
fn cmd_mem(console: &mut Console) {
    console.write_str("memory management not enabled (feature 'mm' disabled)\n");
}

// --- Process subsystem commands ---

#[cfg(feature = "process")]
fn cmd_ps(console: &mut Console) {
    use crate::syscall::impls;

    let fd = match impls::open("/proc") {
        Ok(fd) => fd,
        Err(_) => {
            console.write_str("ps: failed to read /proc\n");
            return;
        }
    };

    let mut pids: Vec<usize> = Vec::new();
    let mut visit = |entry: crate::fs::vfs::DirEntry<'_>| {
        if let Ok(pid) = entry.name.parse::<usize>() {
            pids.push(pid);
        }
        true
    };
    let mut cursor = crate::fs::vfs::DirCursor::default();
    let mut name_buf = [0u8; 32];
    let _ = impls::readdir(fd, &mut cursor, &mut name_buf, &mut visit);
    let _ = impls::close(fd);
    pids.sort_unstable();

    console.write_str("PID  PPID   STATE   NAME\n");
    console.write_str("---  ----   -----   ----\n");

    for pid in &pids {
        let status_path = format!("/proc/{pid}/status");
        let Ok(fd) = impls::open(&status_path) else {
            continue;
        };
        let mut buf = [0u8; 256];
        let mut total = 0usize;
        loop {
            match impls::read(fd, &mut buf[total..]) {
                Ok(0) => break,
                Ok(n) => total += n,
                Err(_) => break,
            }
            if total >= buf.len() { break; }
        }
        let _ = impls::close(fd);
        let Ok(text) = core::str::from_utf8(&buf[..total]) else { continue; };
        let mut name = "";
        let mut state = "";
        let mut ppid = "";
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("Name:\t") { name = v; }
            else if let Some(v) = line.strip_prefix("State:\t") { state = v; }
            else if let Some(v) = line.strip_prefix("PPid:\t") { ppid = v; }
        }
        console.write_str(&format!(
            "{:<4} {:<6} {:<7} {}\n",
            pid, ppid, state, name
        ));
    }
}

#[cfg(not(feature = "process"))]
fn cmd_ps(console: &mut Console) {
    console.write_str("process management not enabled (feature 'process' disabled)\n");
}

// --- Filesystem commands ---

#[cfg(feature = "fs")]
fn cmd_ls(args: &str, console: &mut Console) {
    use crate::syscall::impls;

    let (show_all, show_long, path) = parse_ls_args(args);
    let stat = match impls::stat(path) {
        Ok(stat) => stat,
        Err(errno) => {
            console.write_str(&format!("ls: errno {}\n", errno));
            return;
        }
    };
    if stat.file_type != impls::STAT_TYPE_DIRECTORY {
        if show_long {
            console.write_str(&format!("{}\n", format_stat_entry(path, &stat, "")));
        } else {
            console.write_str(&format!("{} (file, {} bytes)\n", path, stat.size));
        }
        return;
    }

    let fd = match open_dir_fd(path) {
        Ok(fd) => fd,
        Err(_) => {
            console.write_str("ls: failed to read directory\n");
            return;
        }
    };

    let mut cursor = crate::fs::vfs::DirCursor::default();
    let mut name_buf = [0u8; 128];
    let mut seen = 0usize;
    let mut visit = |entry: crate::fs::vfs::DirEntry<'_>| {
        seen += 1;
        if !show_all && (entry.name == "." || entry.name == "..") {
            return true;
        }
        if show_long {
            let entry_path = join_path(path, entry.name);
            match impls::stat(&entry_path) {
                Ok(entry_stat) => {
                    console.write_str(&format!(
                        "{}\n",
                        format_stat_entry(
                            entry.name,
                            &entry_stat,
                            entry_type_indicator(entry.file_type)
                        )
                    ));
                }
                Err(errno) => {
                    console.write_str(&format!("  {}: errno {}\n", entry.name, errno));
                }
            }
        } else {
            let type_indicator = entry_type_indicator(entry.file_type);
            console.write_str(&format!("  {}{}\n", entry.name, type_indicator));
        }
        true
    };
    let result = impls::readdir(fd, &mut cursor, &mut name_buf, &mut visit);
    let _ = impls::close(fd);

    match result {
        Ok(_) if seen == 0 => {
            console.write_str("(empty directory)\n");
        }
        Ok(_) => {}
        Err(e) => console.write_str(&format!("ls: {:?}\n", e)),
    }
}

#[cfg(not(feature = "fs"))]
fn cmd_ls(_args: &str, console: &mut Console) {
    console.write_str("filesystem not enabled (feature 'fs' disabled)\n");
}

#[cfg(feature = "fs")]
fn cmd_cat(args: &str, console: &mut Console) {
    use crate::syscall::impls;

    if args.is_empty() {
        console.write_str("usage: cat <path>\n");
        return;
    }

    let stat = match impls::stat(args) {
        Ok(stat) => stat,
        Err(errno) => {
            console.write_str(&format!("cat: {}: errno {}\n", args, errno));
            return;
        }
    };

    if stat.file_type == impls::STAT_TYPE_DIRECTORY {
        console.write_str(&format!("cat: {}: Is a directory\n", args));
        return;
    }

    match impls::open(args) {
        Ok(fd) => {
            let mut buf = [0u8; 512];
            loop {
                match impls::read(fd, &mut buf) {
                    Ok(0) => break,
                    Ok(n) => console.write_bytes(&buf[..n]),
                    Err(_) => break,
                }
            }
            let _ = impls::close(fd);
            console.write_bytes(b"\n");
        }
        Err(errno) => console.write_str(&format!("cat: {}: errno {}\n", args, errno)),
    }
}

#[cfg(not(feature = "fs"))]
fn cmd_cat(_args: &str, console: &mut Console) {
    console.write_str("filesystem not enabled (feature 'fs' disabled)\n");
}

#[cfg(feature = "fs")]
fn cmd_cd(args: &str, console: &mut Console) {
    use crate::syscall::impls;

    if args.is_empty() {
        console.write_str("usage: cd <path>\n");
        return;
    }

    match impls::chdir(args) {
        Ok(()) => {}
        Err(errno) => console.write_str(&format!("cd: errno {}\n", errno)),
    }
}

#[cfg(not(feature = "fs"))]
fn cmd_cd(_args: &str, console: &mut Console) {
    console.write_str("filesystem not enabled (feature 'fs' disabled)\n");
}

#[cfg(feature = "fs")]
fn cmd_stat(args: &str, console: &mut Console) {
    use crate::syscall::impls;

    if args.is_empty() {
        console.write_str("usage: stat <path>\n");
        return;
    }

    match impls::stat(args) {
        Ok(stat) => {
            console.write_str(&format!("{}\n", format_stat_entry(args, &stat, "")));
        }
        Err(errno) => console.write_str(&format!("stat: errno {}\n", errno)),
    }
}

#[cfg(not(feature = "fs"))]
fn cmd_stat(_args: &str, console: &mut Console) {
    console.write_str("filesystem not enabled (feature 'fs' disabled)\n");
}

#[cfg(feature = "fs")]
fn cmd_mkdir(args: &str, console: &mut Console) {
    use crate::syscall::impls;

    if args.is_empty() {
        console.write_str("usage: mkdir <path>\n");
        return;
    }

    match impls::mkdir(args) {
        Ok(_) => console.write_str(&format!("created directory: {}\n", args)),
        Err(errno) => console.write_str(&format!("mkdir: errno {}\n", errno)),
    }
}

#[cfg(not(feature = "fs"))]
fn cmd_mkdir(_args: &str, console: &mut Console) {
    console.write_str("filesystem not enabled (feature 'fs' disabled)\n");
}

#[cfg(feature = "fs")]
fn cmd_write(args: &str, console: &mut Console) {
    use crate::syscall::impls;

    let parts: Vec<&str> = args.splitn(2, ' ').collect();
    if parts.len() < 2 {
        console.write_str("usage: write <path> <data>\n");
        return;
    }
    let path = parts[0];
    let data = parts[1];

    match impls::write_file(path, data.as_bytes()) {
        Ok(n) => console.write_str(&format!("wrote {} bytes to {}\n", n, path)),
        Err(errno) => console.write_str(&format!("write: errno {}\n", errno)),
    }
}

#[cfg(not(feature = "fs"))]
fn cmd_write(_args: &str, console: &mut Console) {
    console.write_str("filesystem not enabled (feature 'fs' disabled)\n");
}

// --- Device driver commands ---

#[cfg(feature = "drivers")]
fn cmd_lsdev(console: &mut Console) {
    use crate::syscall::impls;

    let fd = match impls::open("/dev") {
        Ok(fd) => fd,
        Err(_) => {
            console.write_str("lsdev: failed to read /dev\n");
            return;
        }
    };
    console.write_str("Registered devices:\n");
    let mut seen = 0usize;
    let mut cursor = crate::fs::vfs::DirCursor::default();
    let mut name_buf = [0u8; 64];
    let mut visit = |entry: crate::fs::vfs::DirEntry<'_>| {
        if entry.name == "." || entry.name == ".." {
            return true;
        }
        seen += 1;
        console.write_str(&format!("  {}\n", entry.name));
        true
    };
    let _ = impls::readdir(fd, &mut cursor, &mut name_buf, &mut visit);
    let _ = impls::close(fd);
    if seen == 0 {
        console.write_str("  (none)\n");
    }
}

#[cfg(not(feature = "drivers"))]
fn cmd_lsdev(console: &mut Console) {
    console.write_str("drivers not enabled (feature 'drivers' disabled)\n");
}

#[cfg(feature = "drivers")]
fn cmd_lspci(console: &mut Console) {
    let buses = crate::drivers::registry::list_buses();
    if buses.is_empty() {
        console.write_str("no buses registered\n");
        return;
    }

    for bus in buses {
        console.write_str(&format!("bus: {}\n", bus));
        if let Some(devices) = crate::drivers::registry::list_bus_devices(bus.as_str()) {
            for device in devices {
                let class_info = match (device.class_code, device.subclass, device.prog_if) {
                    (Some(class_code), Some(subclass), Some(prog_if)) => {
                        format!("class={:02x} subclass={:02x} prog_if={:02x}", class_code, subclass, prog_if)
                    }
                    _ => String::from("class=unknown"),
                };
                let location = match (device.pci_bus, device.pci_device, device.pci_function) {
                    (Some(bus), Some(dev), Some(func)) => format!("{:02x}:{:02x}.{}", bus, dev, func),
                    _ => String::from("unknown"),
                };
                let bar5 = device
                    .bar5
                    .map(|value| format!("bar5=0x{:x}", value))
                    .unwrap_or_else(|| String::from("bar5=unknown"));
                console.write_str(&format!("  {:<10} {} {} {}\n", device.name, location, class_info, bar5));
            }
        }
    }
}

#[cfg(not(feature = "drivers"))]
fn cmd_lspci(console: &mut Console) {
    console.write_str("drivers not enabled (feature 'drivers' disabled)\n");
}

// --- Module commands ---

#[cfg(feature = "modules")]
fn cmd_lsmod(console: &mut Console) {
    use crate::syscall::impls;

    let fd = match impls::open("/proc/modules") {
        Ok(fd) => fd,
        Err(_) => {
            console.write_str("lsmod: failed to read /proc/modules\n");
            return;
        }
    };
    console.write_str("Loaded modules:\n");
    let mut seen = 0usize;
    let mut cursor = crate::fs::vfs::DirCursor::default();
    let mut name_buf = [0u8; 64];
    let mut visit = |entry: crate::fs::vfs::DirEntry<'_>| {
        if entry.name == "." || entry.name == ".." {
            return true;
        }
        seen += 1;
        console.write_str(&format!("  {}\n", entry.name));
        true
    };
    let _ = impls::readdir(fd, &mut cursor, &mut name_buf, &mut visit);
    let _ = impls::close(fd);
    if seen == 0 {
        console.write_str("  (none)\n");
    }
}

#[cfg(not(feature = "modules"))]
fn cmd_lsmod(console: &mut Console) {
    console.write_str("modules not enabled (feature 'modules' disabled)\n");
}

// --- Misc ---

fn cmd_uptime(console: &mut Console) {
    use crate::syscall::impls;

    let ticks = impls::times();
    console.write_str(&format!("ticks since boot: {}\n", ticks));
}

#[cfg(feature = "fs")]
fn parse_ls_args(args: &str) -> (bool, bool, &str) {
    let mut show_long = false;
    let mut show_all = false;
    let mut path = ".";

    for token in args.split_whitespace() {
        if token == "-l" {
            show_long = true;
        } else if token == "-a" {
            show_all = true;
        } else {
            path = token;
        }
    }

    (show_all, show_long, path)
}

#[cfg(feature = "fs")]
fn entry_type_indicator(file_type: crate::fs::vfs::FileType) -> &'static str {
    match file_type {
        crate::fs::vfs::FileType::Directory => "/",
        crate::fs::vfs::FileType::Symlink => "@",
        crate::fs::vfs::FileType::CharDevice => "%",
        crate::fs::vfs::FileType::BlockDevice => "#",
        _ => "",
    }
}

#[cfg(feature = "fs")]
fn format_stat_entry(
    name: &str,
    stat: &crate::syscall::impls::Stat,
    type_suffix: &str,
) -> alloc::string::String {
    let file_type = match stat.file_type {
        crate::syscall::impls::STAT_TYPE_REGULAR => "file",
        crate::syscall::impls::STAT_TYPE_DIRECTORY => "dir",
        crate::syscall::impls::STAT_TYPE_CHAR_DEVICE => "char",
        crate::syscall::impls::STAT_TYPE_BLOCK_DEVICE => "block",
        crate::syscall::impls::STAT_TYPE_SYMLINK => "symlink",
        _ => "unknown",
    };

    format!(
        "{:<20} ino={:<8} size={:<8} type={}",
        format!("{name}{type_suffix}"),
        stat.ino,
        stat.size,
        file_type
    )
}

#[cfg(feature = "fs")]
fn join_path(base: &str, name: &str) -> alloc::string::String {
    if base == "/" {
        return format!("/{}", name);
    }
    if base == "." {
        return format!("./{}", name);
    }

    format!("{}/{}", base.trim_end_matches('/'), name)
}

#[cfg(feature = "fs")]
fn open_dir_fd(path: &str) -> Result<usize, isize> {
    use crate::syscall::impls;

    impls::open(path)
}
