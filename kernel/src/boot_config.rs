extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec;

#[cfg(feature = "fs")]
use crate::fs::vfs::FileSystem;
#[cfg(feature = "arch-x86_64")]
use bootloader_api::BootInfo;
#[cfg(feature = "arch-x86_64")]
use bootloader_api::info::Optional;

use crate::boot_settings::{BootSettings, MAX_CONFIG_BYTES};
pub use crate::boot_settings::{DEFAULT_VIRTUAL_CONSOLES, MAX_VIRTUAL_CONSOLES};

pub struct BootConfig {
    pub settings: BootSettings,
    pub log_level: log::LevelFilter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellConsole {
    Auto,
    Serial,
    Framebuffer,
}

impl BootConfig {
    #[cfg(feature = "arch-x86_64")]
    pub fn from_boot_info(boot_info: &BootInfo) -> Self {
        let result =
            read_ramdisk_file(boot_info, "boot.toml").and_then(|contents| match contents {
                Some(contents) => BootSettings::parse(&contents),
                None => Ok(BootSettings::default()),
            });
        let settings = result.unwrap_or_else(|error| {
            crate::logger::init(None, Some(0x3f8), log::LevelFilter::Info);
            panic!("invalid boot configuration: {error}");
        });
        let log_level = settings.log_level.parse().expect("validated log level");
        Self {
            settings,
            log_level,
        }
    }

    pub fn shell_port(&self) -> u8 {
        self.settings.shell_port
    }

    pub fn shell_console(&self) -> ShellConsole {
        match self.settings.shell_console.as_str() {
            "serial" => ShellConsole::Serial,
            "framebuffer" => ShellConsole::Framebuffer,
            _ => ShellConsole::Auto,
        }
    }

    pub fn virtual_consoles(&self) -> usize {
        self.settings.virtual_consoles
    }

    pub fn syscall_trace(&self) -> bool {
        self.settings.syscall_trace
    }
}

#[cfg(feature = "arch-x86_64")]
pub fn read_ramdisk_file(boot_info: &BootInfo, name: &str) -> Result<Option<String>, String> {
    #[cfg(feature = "fs")]
    {
        let address = match boot_info.ramdisk_addr {
            Optional::Some(address) if boot_info.ramdisk_len > 0 => address,
            _ => return Ok(None),
        };
        // BootInfo provides a mapped ramdisk that lives for the entire boot.
        let ramdisk = unsafe {
            core::slice::from_raw_parts(address as *const u8, boot_info.ramdisk_len as usize)
        };
        let fs = crate::fs::initrd::InitrdFs::new(ramdisk)
            .map_err(|e| format!("invalid boot ramdisk: {e:?}"))?;
        let mut inode = fs.root_inode();
        for component in name.split('/') {
            inode = match inode.lookup(component) {
                Ok(inode) => inode,
                Err(crate::fs::vfs::FsError::NotFound) => return Ok(None),
                Err(e) => return Err(format!("cannot read ramdisk {name}: {e:?}")),
            };
        }
        read_config_inode(inode.as_ref()).map(Some)
    }
    #[cfg(not(feature = "fs"))]
    {
        let _ = (boot_info, name);
        Ok(None)
    }
}

#[cfg(feature = "fs")]
pub fn read_config_inode(inode: &dyn crate::fs::vfs::Inode) -> Result<String, String> {
    if inode.file_type() != crate::fs::vfs::FileType::Regular || inode.size() > MAX_CONFIG_BYTES {
        return Err("configuration must be a regular file of at most 16 KiB".into());
    }
    let mut bytes = vec![0; MAX_CONFIG_BYTES + 1];
    let mut length = 0;
    loop {
        let count = inode
            .read(length, &mut bytes[length..])
            .map_err(|e| format!("configuration read failed: {e:?}"))?;
        length += count;
        if length > MAX_CONFIG_BYTES {
            return Err("configuration exceeds 16 KiB".into());
        }
        if count == 0 {
            bytes.truncate(length);
            return String::from_utf8(bytes).map_err(|_| "configuration is not UTF-8".into());
        }
    }
}
