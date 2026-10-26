use alloc::format;
use alloc::string::String;

use super::{MountDevice, mount_required, vfs};
use crate::boot_config::{read_config_inode, read_ramdisk_file};
use crate::boot_settings::{BootSettings, parse_fstab, validate_fstab_root};

#[cfg(feature = "arch-x86_64")]
pub fn mount_filesystems(
    settings: &BootSettings,
    boot_info: &bootloader_api::BootInfo,
) -> Result<(), String> {
    mount_required("/", "ramfs", MountDevice::None)
        .map_err(|e| format!("bootstrap root: {e:?}"))?;
    if settings.root_fs != "ramfs" {
        mount_required(
            "/",
            &settings.root_fs,
            MountDevice::device_path(&settings.root_device),
        )
        .map_err(|e| {
            format!(
                "root {} on {}: {e:?}",
                settings.root_fs, settings.root_device
            )
        })?;
    }

    let contents = match vfs::lookup_path_with_options("/etc/fstab", true) {
        Ok(inode) => read_config_inode(inode.as_ref())?,
        Err(vfs::FsError::NotFound) => {
            let contents = read_ramdisk_file(boot_info, "etc/fstab")?
                .ok_or_else(|| String::from("no /etc/fstab in root or boot ramdisk"))?;
            if settings.root_fs == "ramfs" {
                vfs::create_path("/etc", vfs::FileType::Directory)
                    .map_err(|e| format!("RAM root /etc: {e:?}"))?;
                let file = vfs::create_path("/etc/fstab", vfs::FileType::Regular)
                    .map_err(|e| format!("RAM root /etc/fstab: {e:?}"))?;
                let written = file
                    .write(0, contents.as_bytes())
                    .map_err(|e| format!("RAM root fstab write: {e:?}"))?;
                if written != contents.len() {
                    return Err("short write installing RAM root fstab".into());
                }
                log::info!("fstab: installed boot ramdisk defaults in RAM root");
            } else {
                log::warn!("fstab: /etc/fstab missing; using boot ramdisk fallback");
            }
            contents
        }
        Err(e) => return Err(format!("/etc/fstab lookup failed: {e:?}")),
    };
    let entries = parse_fstab(&contents)?;
    validate_fstab_root(&entries, settings)?;
    for entry in entries {
        if entry.target == "/" {
            log::info!("fstab: root entry matches early boot settings");
            continue;
        }
        if entry.noauto {
            log::info!("fstab: skipping noauto mount '{}'", entry.target);
            continue;
        }
        let device = if entry.source.starts_with("/dev/") {
            MountDevice::device_path(&entry.source)
        } else {
            MountDevice::None
        };
        mount_required(&entry.target, &entry.filesystem, device).map_err(|e| {
            format!(
                "fstab: {} on {} ({}): {e:?}",
                entry.source, entry.target, entry.filesystem
            )
        })?;
    }
    log::info!("fstab: configured mounts ready");
    Ok(())
}
