extern crate alloc;

use core::slice;

#[cfg(feature = "arch-x86_64")]
use bootloader_api::BootInfo;
#[cfg(feature = "arch-x86_64")]
use bootloader_api::info::Optional;

#[cfg(feature = "fs")]
use crate::fs::initrd::InitrdFs;
#[cfg(feature = "fs")]
use crate::fs::vfs::FileSystem;
#[cfg(feature = "arch-x86_64")]
use x86_64::VirtAddr;

pub struct BootConfig {
    pub log_level: log::LevelFilter,
    pub shell_port: u8,
    pub shell_console: ShellConsole,
    pub virtual_consoles: usize,
    pub syscall_trace: bool,
}

pub const DEFAULT_VIRTUAL_CONSOLES: usize = 6;
pub const MAX_VIRTUAL_CONSOLES: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellConsole {
    Auto,
    Serial,
    Framebuffer,
}

impl Default for BootConfig {
    fn default() -> Self {
        Self {
            log_level: log::LevelFilter::Info,
            shell_port: 1,
            shell_console: ShellConsole::Auto,
            virtual_consoles: DEFAULT_VIRTUAL_CONSOLES,
            syscall_trace: false,
        }
    }
}

impl BootConfig {
    #[cfg(feature = "arch-x86_64")]
    pub fn from_boot_info(boot_info: &BootInfo) -> Self {
        let mut config = Self::default();
        config.log_level = read_log_level(boot_info).unwrap_or(config.log_level);
        config.shell_port = read_shell_port(boot_info).unwrap_or(config.shell_port);
        config.shell_console = read_shell_console(boot_info).unwrap_or(config.shell_console);
        config.virtual_consoles =
            read_virtual_consoles(boot_info).unwrap_or(config.virtual_consoles);
        config.syscall_trace = read_syscall_trace(boot_info).unwrap_or(config.syscall_trace);
        config
    }

    pub fn shell_port(&self) -> u8 {
        self.shell_port
    }

    pub fn shell_console(&self) -> ShellConsole {
        self.shell_console
    }

    pub fn virtual_consoles(&self) -> usize {
        self.virtual_consoles
    }

    pub fn syscall_trace(&self) -> bool {
        self.syscall_trace
    }
}

#[cfg(feature = "arch-x86_64")]
fn read_syscall_trace(boot_info: &BootInfo) -> Option<bool> {
    let ramdisk_addr = match boot_info.ramdisk_addr {
        Optional::Some(addr) if boot_info.ramdisk_len > 0 => addr,
        _ => return None,
    };

    let len = boot_info.ramdisk_len as usize;
    let ptr = VirtAddr::new(ramdisk_addr).as_ptr();
    let ramdisk = unsafe {
        // SAFETY: the bootloader already maps the ramdisk into virtual memory and
        // exposes the mapped address in BootInfo::ramdisk_addr.
        slice::from_raw_parts(ptr, len)
    };

    let fs = InitrdFs::new(ramdisk).ok()?;
    let root = fs.root_inode();
    let config = root.lookup("boot.toml").ok()?;

    let mut buf = [0u8; 256];
    let read = config.read(0, &mut buf).ok()?;
    let contents = core::str::from_utf8(&buf[..read]).ok()?;
    parse_syscall_trace(contents)
}

#[cfg(feature = "arch-x86_64")]
fn read_log_level(boot_info: &BootInfo) -> Option<log::LevelFilter> {
    let ramdisk_addr = match boot_info.ramdisk_addr {
        Optional::Some(addr) if boot_info.ramdisk_len > 0 => addr,
        _ => return None,
    };

    let len = boot_info.ramdisk_len as usize;
    let ptr = VirtAddr::new(ramdisk_addr).as_ptr();
    let ramdisk = unsafe {
        // SAFETY: the bootloader already maps the ramdisk into virtual memory and
        // exposes the mapped address in BootInfo::ramdisk_addr.
        slice::from_raw_parts(ptr, len)
    };

    let fs = InitrdFs::new(ramdisk).ok()?;
    let root = fs.root_inode();
    let config = root.lookup("boot.toml").ok()?;

    let mut buf = [0u8; 256];
    let read = config.read(0, &mut buf).ok()?;
    let contents = core::str::from_utf8(&buf[..read]).ok()?;
    parse_log_level(contents)
}

#[cfg(feature = "arch-x86_64")]
fn read_shell_port(boot_info: &BootInfo) -> Option<u8> {
    let ramdisk_addr = match boot_info.ramdisk_addr {
        Optional::Some(addr) if boot_info.ramdisk_len > 0 => addr,
        _ => return None,
    };

    let len = boot_info.ramdisk_len as usize;
    let ptr = VirtAddr::new(ramdisk_addr).as_ptr();
    let ramdisk = unsafe {
        // SAFETY: the bootloader already maps the ramdisk into virtual memory and
        // exposes the mapped address in BootInfo::ramdisk_addr.
        slice::from_raw_parts(ptr, len)
    };

    let fs = InitrdFs::new(ramdisk).ok()?;
    let root = fs.root_inode();
    let config = root.lookup("boot.toml").ok()?;

    let mut buf = [0u8; 256];
    let read = config.read(0, &mut buf).ok()?;
    let contents = core::str::from_utf8(&buf[..read]).ok()?;
    parse_shell_port(contents)
}

#[cfg(feature = "arch-x86_64")]
fn read_shell_console(boot_info: &BootInfo) -> Option<ShellConsole> {
    let ramdisk_addr = match boot_info.ramdisk_addr {
        Optional::Some(addr) if boot_info.ramdisk_len > 0 => addr,
        _ => return None,
    };

    let len = boot_info.ramdisk_len as usize;
    let ptr = VirtAddr::new(ramdisk_addr).as_ptr();
    let ramdisk = unsafe {
        // SAFETY: the bootloader already maps the ramdisk into virtual memory and
        // exposes the mapped address in BootInfo::ramdisk_addr.
        slice::from_raw_parts(ptr, len)
    };

    let fs = InitrdFs::new(ramdisk).ok()?;
    let root = fs.root_inode();
    let config = root.lookup("boot.toml").ok()?;

    let mut buf = [0u8; 256];
    let read = config.read(0, &mut buf).ok()?;
    let contents = core::str::from_utf8(&buf[..read]).ok()?;
    parse_shell_console(contents)
}

fn read_virtual_consoles(boot_info: &BootInfo) -> Option<usize> {
    #[cfg(feature = "arch-x86_64")]
    {
        let ramdisk_addr = match boot_info.ramdisk_addr {
            Optional::Some(addr) if boot_info.ramdisk_len > 0 => addr,
            _ => return None,
        };

        let len = boot_info.ramdisk_len as usize;
        let ptr = VirtAddr::new(ramdisk_addr).as_ptr();
        let ramdisk = unsafe {
            // SAFETY: the bootloader already maps the ramdisk into virtual memory and
            // exposes the mapped address in BootInfo::ramdisk_addr.
            slice::from_raw_parts(ptr, len)
        };

        let fs = InitrdFs::new(ramdisk).ok()?;
        let root = fs.root_inode();
        let config = root.lookup("boot.toml").ok()?;

        let mut buf = [0u8; 256];
        let read = config.read(0, &mut buf).ok()?;
        let contents = core::str::from_utf8(&buf[..read]).ok()?;
        parse_virtual_consoles(contents)
    }

    #[cfg(not(feature = "arch-x86_64"))]
    None
}

fn parse_log_level(contents: &str) -> Option<log::LevelFilter> {
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (key, value) = line.split_once('=')?;
        if key.trim() != "log_level" {
            continue;
        }

        let value = value.trim().trim_matches('"');
        return match value {
            "off" => Some(log::LevelFilter::Off),
            "error" => Some(log::LevelFilter::Error),
            "warn" | "warning" => Some(log::LevelFilter::Warn),
            "info" => Some(log::LevelFilter::Info),
            "debug" => Some(log::LevelFilter::Debug),
            "trace" => Some(log::LevelFilter::Trace),
            _ => None,
        };
    }

    None
}

fn parse_shell_port(contents: &str) -> Option<u8> {
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (key, value) = line.split_once('=')?;
        if key.trim() != "shell_port" {
            continue;
        }

        let value = value.trim().trim_matches('"');
        return value
            .parse::<u8>()
            .ok()
            .filter(|port| *port >= 1 && *port <= 2);
    }

    None
}

fn parse_shell_console(contents: &str) -> Option<ShellConsole> {
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (key, value) = line.split_once('=')?;
        if key.trim() != "shell_console" {
            continue;
        }

        let value = value.trim().trim_matches('"');
        return match value {
            "auto" => Some(ShellConsole::Auto),
            "serial" => Some(ShellConsole::Serial),
            "framebuffer" => Some(ShellConsole::Framebuffer),
            _ => None,
        };
    }

    None
}

fn parse_syscall_trace(contents: &str) -> Option<bool> {
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (key, value) = line.split_once('=')?;
        if key.trim() != "syscall_trace" {
            continue;
        }

        let value = value.trim().trim_matches('"').to_ascii_lowercase();
        return match value.as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        };
    }

    None
}

fn parse_virtual_consoles(contents: &str) -> Option<usize> {
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (key, value) = line.split_once('=')?;
        if key.trim() != "virtual_consoles" {
            continue;
        }

        let value = value.trim().trim_matches('"');
        return value
            .parse::<usize>()
            .ok()
            .map(|count| count.clamp(1, MAX_VIRTUAL_CONSOLES));
    }

    None
}
