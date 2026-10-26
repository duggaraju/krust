extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub const MAX_CONFIG_BYTES: usize = 16 * 1024;
pub const DEFAULT_VIRTUAL_CONSOLES: usize = 6;
pub const MAX_VIRTUAL_CONSOLES: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootSettings {
    pub log_level: String,
    pub shell_port: u8,
    pub shell_console: String,
    pub virtual_consoles: usize,
    pub syscall_trace: bool,
    pub root_fs: String,
    pub root_device: String,
}

impl Default for BootSettings {
    fn default() -> Self {
        Self {
            log_level: "info".into(),
            shell_port: 1,
            shell_console: "auto".into(),
            virtual_consoles: DEFAULT_VIRTUAL_CONSOLES,
            syscall_trace: false,
            root_fs: "ext4".into(),
            root_device: "/dev/sda".into(),
        }
    }
}

impl BootSettings {
    pub fn parse(contents: &str) -> Result<Self, String> {
        Self::parse_with_defaults(contents, Self::default())
    }

    pub fn parse_with_defaults(contents: &str, mut settings: Self) -> Result<Self, String> {
        if contents.len() > MAX_CONFIG_BYTES {
            return Err("boot configuration exceeds 16 KiB".into());
        }
        let mut keys = Vec::new();
        for (index, line) in contents.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let error = |reason: &str| format!("boot configuration line {}: {reason}", index + 1);
            let (key, raw) = line
                .split_once('=')
                .ok_or_else(|| error("expected key = value"))?;
            let key = key.trim();
            if keys.contains(&key) {
                return Err(error("duplicate key"));
            }
            keys.push(key);
            let raw = raw.trim();
            let value = if let Some(quoted) = raw.strip_prefix('"') {
                let (value, tail) = quoted
                    .split_once('"')
                    .ok_or_else(|| error("unclosed string"))?;
                if value.contains('\\') || !(tail.trim().is_empty() || tail.trim().starts_with('#'))
                {
                    return Err(error("unsupported string syntax"));
                }
                value
            } else {
                raw.split('#').next().unwrap_or("").trim()
            };
            let number = || {
                value
                    .parse::<usize>()
                    .map_err(|_| error("expected unsigned integer"))
            };
            match key {
                "log_level"
                    if matches!(
                        value,
                        "off" | "error" | "warn" | "warning" | "info" | "debug" | "trace"
                    ) =>
                {
                    settings.log_level = if value == "warning" { "warn" } else { value }.into();
                }
                "shell_port" if matches!(value, "1" | "2") => {
                    settings.shell_port = value.parse().map_err(|_| error("invalid port"))?
                }
                "shell_console" if matches!(value, "auto" | "serial" | "framebuffer") => {
                    settings.shell_console = value.into()
                }
                "virtual_consoles" => {
                    settings.virtual_consoles = number()?.clamp(1, MAX_VIRTUAL_CONSOLES);
                }
                "syscall_trace" => {
                    settings.syscall_trace = match value.to_ascii_lowercase().as_str() {
                        "1" | "true" | "yes" | "on" => true,
                        "0" | "false" | "no" | "off" => false,
                        _ => return Err(error("invalid syscall_trace boolean")),
                    };
                }
                "root_fs" if matches!(value, "ext4" | "fat" | "ramfs") => {
                    settings.root_fs = value.into()
                }
                "root_device" => settings.root_device = value.into(),
                _ => return Err(error("unknown setting or invalid value")),
            }
        }
        settings.validate()?;
        Ok(settings)
    }

    pub fn validate(&self) -> Result<(), String> {
        match self.root_fs.as_str() {
            "ramfs" => {
                if !self.root_device.is_empty() && self.root_device != "none" {
                    return Err("ramfs root requires root_device = \"none\"".into());
                }
            }
            "ext4" | "fat" => {
                if !self.root_device.starts_with("/dev/") || !canonical_path(&self.root_device) {
                    return Err("disk root requires an absolute /dev/... root_device".into());
                }
            }
            _ => return Err("unsupported root filesystem".into()),
        }
        Ok(())
    }

    pub fn to_toml(&self) -> String {
        format!(
            "log_level = \"{}\"\nshell_port = {}\nshell_console = \"{}\"\nvirtual_consoles = {}\nsyscall_trace = {}\nroot_fs = \"{}\"\nroot_device = \"{}\"\n",
            self.log_level,
            self.shell_port,
            self.shell_console,
            self.virtual_consoles,
            self.syscall_trace,
            self.root_fs,
            self.root_device
        )
    }

    pub fn default_fstab(&self) -> String {
        format!(
            "# Generated by krust; root is mounted before this file is read.\n{} / {} defaults 0 0\nproc /proc proc defaults 0 0\ndev /dev dev defaults 0 0\n",
            if self.root_fs == "ramfs" {
                String::from("none")
            } else {
                self.root_device.replace(' ', "\\040")
            },
            self.root_fs
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FstabEntry {
    pub source: String,
    pub target: String,
    pub filesystem: String,
    pub noauto: bool,
}

fn canonical_path(path: &str) -> bool {
    path.trim() == path
        && (path == "/"
            || (path.starts_with('/')
                && !path.ends_with('/')
                && !path
                    .chars()
                    .any(|c| c.is_control() || c == '"' || c == '\\')
                && path[1..]
                    .split('/')
                    .all(|part| !matches!(part, "" | "." | ".."))))
}

fn decode_field(field: &str) -> Result<String, String> {
    let mut value = String::new();
    let mut chars = field.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            value.push(c);
            continue;
        }
        match (chars.next(), chars.next(), chars.next()) {
            (Some('0'), Some('4'), Some('0')) => value.push(' '),
            (Some('0'), Some('1'), Some('1')) => value.push('\t'),
            (Some('1'), Some('3'), Some('4')) => value.push('\\'),
            _ => return Err("unsupported fstab escape".into()),
        }
    }
    Ok(value)
}

pub fn parse_fstab(contents: &str) -> Result<Vec<FstabEntry>, String> {
    if contents.len() > MAX_CONFIG_BYTES {
        return Err("fstab exceeds 16 KiB".into());
    }
    let mut entries: Vec<FstabEntry> = Vec::new();
    for (index, line) in contents.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let error = |reason: &str| format!("fstab line {}: {reason}", index + 1);
        let fields: Vec<_> = line
            .split_whitespace()
            .take_while(|field| !field.starts_with('#'))
            .collect();
        if fields.len() != 6 {
            return Err(error("expected six fields"));
        }
        let source = decode_field(fields[0]).map_err(|e| error(&e))?;
        let target = decode_field(fields[1]).map_err(|e| error(&e))?;
        if !canonical_path(&target) {
            return Err(error("mount target must be a canonical absolute path"));
        }
        if entries.iter().any(|entry| entry.target == target) {
            return Err(error("duplicate mount target"));
        }
        let filesystem = fields[2];
        match filesystem {
            "fat" | "ext4" if source.starts_with("/dev/") && canonical_path(&source) => {}
            "ramfs" | "proc" | "dev" if source == "none" || source == filesystem => {}
            _ => return Err(error("unsupported filesystem or source")),
        }
        let mut noauto = false;
        for option in fields[3].split(',') {
            match option {
                "defaults" => {}
                "noauto" => noauto = true,
                _ => {
                    return Err(error(
                        "unsupported mount option (only defaults and noauto are supported)",
                    ));
                }
            }
        }
        if fields[4] != "0" || fields[5] != "0" {
            return Err(error(
                "dump/fsck are not implemented; final fields must be 0 0",
            ));
        }
        if target == "/" && (noauto || !matches!(filesystem, "ext4" | "fat" | "ramfs")) {
            return Err(error("invalid root entry"));
        }
        if entries.len() >= 64 {
            return Err(error("too many mount entries (maximum 64)"));
        }
        entries.push(FstabEntry {
            source,
            target,
            filesystem: filesystem.into(),
            noauto,
        });
    }
    entries.sort_by_key(|entry| entry.target.matches('/').count());
    Ok(entries)
}

pub fn validate_fstab_root(entries: &[FstabEntry], settings: &BootSettings) -> Result<(), String> {
    if let Some(root) = entries.iter().find(|entry| entry.target == "/") {
        let expected_source = if settings.root_fs == "ramfs" {
            "none"
        } else {
            &settings.root_device
        };
        if root.filesystem != settings.root_fs
            || (root.source != expected_source
                && !(settings.root_fs == "ramfs" && root.source == "ramfs"))
        {
            return Err("fstab root entry disagrees with early boot root settings".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_settings_round_trip_and_root_validation() {
        for fs in ["ext4", "fat", "ramfs"] {
            let settings = BootSettings {
                root_fs: fs.into(),
                root_device: if fs == "ramfs" { "none" } else { "/dev/sda" }.into(),
                ..Default::default()
            };
            assert_eq!(BootSettings::parse(&settings.to_toml()).unwrap(), settings);
            validate_fstab_root(&parse_fstab(&settings.default_fstab()).unwrap(), &settings)
                .unwrap();
        }
        for text in [
            "root_fs = \"unknown\"",
            "root_device = \"sda\"",
            "root_fs = \"ramfs\"",
            "root_fs = \"fat\"\nroot_fs = \"ext4\"",
            "unknown = true",
            "shell_port = 3",
            "syscall_trace = maybe",
            "virtual_consoles = invalid",
            "root_device = \"/dev/sda\" trailing",
            "root_fs = \"ext4",
        ] {
            assert!(BootSettings::parse(text).is_err(), "{text}");
        }
        assert!(BootSettings::parse(&" ".repeat(MAX_CONFIG_BYTES + 1)).is_err());
        let large = format!(
            "{}root_fs = \"fat\"\n",
            "# configuration comment\n".repeat(100)
        );
        assert_eq!(BootSettings::parse(&large).unwrap().root_fs, "fat");
        let compatible = BootSettings::parse(
            "log_level = \"warning\"\nsyscall_trace = \"YES\"\nvirtual_consoles = 99",
        )
        .unwrap();
        assert_eq!(compatible.log_level, "warn");
        assert!(compatible.syscall_trace);
        assert_eq!(compatible.virtual_consoles, MAX_VIRTUAL_CONSOLES);
    }

    #[test]
    fn fstab_orders_parents_and_handles_comments_noauto_and_spaces() {
        let entries = parse_fstab(
            "none /mnt/child ramfs defaults 0 0\nnone /mnt ramfs defaults 0 0\n\
             proc /proc proc defaults 0 0 # comment\n/dev/sdb /other\\040disk fat noauto 0 0\n",
        )
        .unwrap();
        assert_eq!(entries.last().unwrap().target, "/mnt/child");
        assert!(
            entries
                .iter()
                .any(|e| e.target == "/other disk" && e.noauto)
        );
    }

    #[test]
    fn invalid_fstab_is_rejected_before_mounting() {
        for text in [
            "proc /proc proc defaults 0",
            "proc /proc proc defaults 0 1",
            "proc /proc proc ro 0 0",
            "proc relative proc defaults 0 0",
            "proc /a/../b proc defaults 0 0",
            "none / ramfs noauto 0 0",
            "proc /proc proc defaults 0 0\nproc /proc proc defaults 0 0",
            "UUID=abc / ext4 defaults 0 0",
            "none /mnt unknown defaults 0 0",
        ] {
            assert!(parse_fstab(text).is_err(), "{text}");
        }
        let settings = BootSettings::default();
        assert!(
            validate_fstab_root(
                &parse_fstab("/dev/sdb / fat defaults 0 0").unwrap(),
                &settings
            )
            .is_err()
        );
    }

    #[test]
    fn configuration_limits_and_escaped_device_round_trip() {
        let mut contents = String::new();
        for index in 0..64 {
            contents.push_str(&format!("none /mount{index} ramfs defaults 0 0\n"));
        }
        assert_eq!(parse_fstab(&contents).unwrap().len(), 64);
        contents.push_str("none /extra ramfs defaults 0 0\n");
        assert!(parse_fstab(&contents).unwrap_err().contains("too many"));
        assert!(parse_fstab(&" ".repeat(MAX_CONFIG_BYTES + 1)).is_err());
        let settings = BootSettings::parse("root_device = \"/dev/my disk\"").unwrap();
        validate_fstab_root(&parse_fstab(&settings.default_fstab()).unwrap(), &settings).unwrap();
        assert!(parse_fstab("none /mount\\040 ramfs defaults 0 0").is_err());
    }
}
