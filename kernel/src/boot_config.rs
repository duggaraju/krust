use core::slice;

use bootloader_api::BootInfo;
use bootloader_api::info::Optional;
use bootloader_boot_config::LevelFilter;

use crate::fs::initrd::InitrdFs;
use crate::fs::vfs::FileSystem;
use x86_64::VirtAddr;

pub struct BootConfig {
    pub log_level: LevelFilter,
    pub shell_port: u8,
    pub shell_console: ShellConsole,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellConsole {
    Auto,
    Serial,
    Framebuffer,
}

impl Default for BootConfig {
    fn default() -> Self {
        Self {
            log_level: LevelFilter::Info,
            shell_port: 1,
            shell_console: ShellConsole::Auto,
        }
    }
}

impl BootConfig {
    pub fn from_boot_info(boot_info: &BootInfo) -> Self {
        let mut config = Self::default();
        config.log_level = read_log_level(boot_info).unwrap_or(config.log_level);
        config.shell_port = read_shell_port(boot_info).unwrap_or(config.shell_port);
        config.shell_console = read_shell_console(boot_info).unwrap_or(config.shell_console);
        config
    }

    pub fn to_log_level(&self) -> log::LevelFilter {
        match self.log_level {
            LevelFilter::Off => log::LevelFilter::Off,
            LevelFilter::Error => log::LevelFilter::Error,
            LevelFilter::Warn => log::LevelFilter::Warn,
            LevelFilter::Info => log::LevelFilter::Info,
            LevelFilter::Debug => log::LevelFilter::Debug,
            LevelFilter::Trace => log::LevelFilter::Trace,
        }
    }

    pub fn shell_port(&self) -> u8 {
        self.shell_port
    }

    pub fn shell_console(&self) -> ShellConsole {
        self.shell_console
    }
}

fn read_log_level(boot_info: &BootInfo) -> Option<LevelFilter> {
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

fn parse_log_level(contents: &str) -> Option<LevelFilter> {
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
            "off" => Some(LevelFilter::Off),
            "error" => Some(LevelFilter::Error),
            "warn" | "warning" => Some(LevelFilter::Warn),
            "info" => Some(LevelFilter::Info),
            "debug" => Some(LevelFilter::Debug),
            "trace" => Some(LevelFilter::Trace),
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
