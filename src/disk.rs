use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::process::{Command, Stdio};

// ── FAT32 VHD ─────────────────────────────────────────────────────────────────

/// Create a FAT32 VHD by recursively copying all files from `source_dir`.
pub fn create_fat_vhd_from_dir(
    source_dir: &Path,
    output: &Path,
    size_mib: u64,
    exclude: HashSet<&str>,
) -> io::Result<()> {
    let data_size = size_mib * 1024 * 1024;
    if data_size < 4 * 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "VHD size must be at least 4 MiB",
        ));
    }

    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }

    {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(output)?;
        file.set_len(data_size)?;

        fatfs::format_volume(&mut file, fatfs::FormatVolumeOptions::new())
            .map_err(io::Error::other)?;
        file.seek(SeekFrom::Start(0))?;

        let fs =
            fatfs::FileSystem::new(&mut file, fatfs::FsOptions::new()).map_err(io::Error::other)?;
        let root = fs.root_dir();

        copy_dir_to_fat(source_dir, source_dir, &root, &exclude)?;
    }

    append_vhd_footer(output, data_size)?;
    Ok(())
}

fn copy_dir_to_fat<T: fatfs::ReadWriteSeek>(
    base: &Path,
    dir: &Path,
    fat_dir: &fatfs::Dir<'_, T>,
    exclude: &HashSet<&str>,
) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let host_path = entry.path();
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();

        if exclude.contains(name.as_ref()) {
            continue;
        }

        if host_path.is_dir() {
            let sub = fat_dir
                .create_dir(name.as_ref())
                .map_err(io::Error::other)?;
            copy_dir_to_fat(base, &host_path, &sub, exclude)?;
        } else if host_path.is_symlink() {
            // FAT has no symlinks; skip silently
        } else {
            let mut data = Vec::new();
            File::open(&host_path)?.read_to_end(&mut data)?;

            fat_dir
                .create_file(name.as_ref())
                .map_err(io::Error::other)?
                .write_all(&data)
                .map_err(io::Error::other)?;

            let rel = host_path.strip_prefix(base).unwrap_or(&host_path);
            eprintln!("  wrote /{} ({} bytes)", rel.display(), data.len());
        }
    }
    Ok(())
}

// ── ext4 VHD ──────────────────────────────────────────────────────────────────

/// Create an ext4 VHD by recursively copying all files from `source_dir`.
///
/// Requires `mkfs.ext4` and `debugfs` from e2fsprogs.
/// Symlinks are preserved; hard links are copied as independent files.
pub fn create_ext4_vhd_from_dir(
    source_dir: &Path,
    output: &Path,
    size_mib: u64,
    exclude: HashSet<&str>,
) -> io::Result<()> {
    let data_size = size_mib * 1024 * 1024;
    if data_size < 16 * 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ext4 VHD size must be at least 16 MiB",
        ));
    }

    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }

    // Allocate the raw image file.
    {
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(output)?;
        file.set_len(data_size)?;
    }

    // Format with mkfs.ext4 (-b 4096 for compatibility with the kernel's ext4_rs driver).
    let status = Command::new("mkfs.ext4")
        .args(["-b", "4096", "-F"])
        .arg(output)
        .status()
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("mkfs.ext4 not found — install e2fsprogs: {}", e),
            )
        })?;
    if !status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("mkfs.ext4 exited with {}", status),
        ));
    }

    // Build a debugfs script to populate the filesystem without root.
    // debugfs supports: mkdir, write <src> <dst>, symlink <dst> <target>.
    let mut script = String::new();
    collect_debugfs_cmds(source_dir, source_dir, &exclude, &mut script)?;

    let status = Command::new("debugfs")
        .args(["-w", "-f", "/dev/stdin"])
        .arg(output)
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("debugfs not found — install e2fsprogs: {}", e),
            )
        })
        .and_then(|mut child| {
            child.stdin.take().unwrap().write_all(script.as_bytes())?;
            child.wait()
        })?;

    if !status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("debugfs exited with {}", status),
        ));
    }

    // Ensure the file is exactly data_size bytes before appending the footer.
    // mkfs.ext4 or debugfs could have changed the file size.
    OpenOptions::new()
        .write(true)
        .open(output)?
        .set_len(data_size)?;

    append_vhd_footer(output, data_size)?;
    Ok(())
}

/// Recursively walk `dir` and emit debugfs commands into `script`.
///
/// `debugfs` command reference:
/// - `mkdir /path`                         — create directory
/// - `write <host_path> /dest`             — copy file into image
/// - `symlink /link_path <target>`         — create symlink
fn collect_debugfs_cmds(
    base: &Path,
    dir: &Path,
    exclude: &HashSet<&str>,
    script: &mut String,
) -> io::Result<()> {
    // Sort entries for deterministic output.
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let host_path = entry.path();
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();

        if exclude.contains(name.as_ref()) {
            continue;
        }

        let rel = host_path
            .strip_prefix(base)
            .unwrap_or(&host_path)
            .to_string_lossy()
            .into_owned();
        let img_path = format!("/{}", rel.replace('\\', "/"));

        let meta = fs::symlink_metadata(&host_path)?;

        if meta.file_type().is_symlink() {
            let target = fs::read_link(&host_path)?;
            let target_str = target.to_string_lossy();
            // debugfs: symlink <link_in_image> <target>
            script.push_str(&format!("symlink {} {}\n", img_path, target_str));
            eprintln!("  symlink {} → {}", img_path, target_str);
        } else if meta.is_dir() {
            script.push_str(&format!("mkdir {}\n", img_path));
            collect_debugfs_cmds(base, &host_path, exclude, script)?;
        } else if meta.is_file() {
            // debugfs write needs an absolute host path.
            let abs = fs::canonicalize(&host_path)?;
            script.push_str(&format!("write {} {}\n", abs.display(), img_path));
            eprintln!("  wrote {} ({} bytes)", img_path, meta.len());
        }
    }
    Ok(())
}

// Keep old name as an alias so any external callers aren't broken.
#[allow(dead_code)]
pub fn create_vhd_from_dir(
    source_dir: &Path,
    output: &Path,
    size_mib: u64,
    exclude: HashSet<&str>,
) -> io::Result<()> {
    create_fat_vhd_from_dir(source_dir, output, size_mib, exclude)
}

// ── VHD Fixed Disk Footer (Microsoft VHD spec) ────────────────────────────────

fn append_vhd_footer(path: &Path, data_size: u64) -> io::Result<()> {
    let mut footer = [0u8; 512];

    footer[0..8].copy_from_slice(b"conectix");
    put_u32_be(&mut footer, 8, 0x0000_0002); // features
    put_u32_be(&mut footer, 12, 0x0001_0000); // file format version
    put_u64_be(&mut footer, 16, 0xFFFF_FFFF_FFFF_FFFF); // data offset (fixed)
    put_u32_be(&mut footer, 24, vhd_timestamp());
    footer[28..32].copy_from_slice(b"KRST"); // creator app
    put_u32_be(&mut footer, 32, 0x0001_0000); // creator version
    footer[36..40].copy_from_slice(b"Wi2k"); // creator OS
    put_u64_be(&mut footer, 40, data_size); // original size
    put_u64_be(&mut footer, 48, data_size); // current size

    let (cylinders, heads, sectors) = chs_geometry(data_size);
    put_u16_be(&mut footer, 56, cylinders);
    footer[58] = heads;
    footer[59] = sectors;

    put_u32_be(&mut footer, 60, 2); // disk type: fixed
    footer[68..84].copy_from_slice(&pseudo_uuid());
    footer[84] = 0; // saved state

    let checksum = vhd_checksum(&footer);
    put_u32_be(&mut footer, 64, checksum);

    OpenOptions::new()
        .append(true)
        .open(path)?
        .write_all(&footer)?;
    Ok(())
}

fn vhd_checksum(footer: &[u8; 512]) -> u32 {
    let sum: u32 = footer
        .iter()
        .enumerate()
        .filter(|(i, _)| !(64..68).contains(i))
        .fold(0u32, |acc, (_, &b)| acc.wrapping_add(b as u32));
    !sum
}

fn vhd_timestamp() -> u32 {
    const VHD_EPOCH: u64 = 946_684_800; // 2000-01-01 00:00:00 UTC
    let unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    unix.saturating_sub(VHD_EPOCH) as u32
}

fn pseudo_uuid() -> [u8; 16] {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id() as u128;
    let mut value = now ^ (pid << 64);
    let mut out = [0u8; 16];
    for byte in &mut out {
        *byte = (value & 0xFF) as u8;
        value = value.rotate_left(7) ^ 0xA5A5_A5A5_A5A5_A5A5_u128;
    }
    out[6] = (out[6] & 0x0F) | 0x40; // version 4
    out[8] = (out[8] & 0x3F) | 0x80; // RFC 4122 variant
    out
}

fn chs_geometry(disk_size: u64) -> (u16, u8, u8) {
    let total_sectors = (disk_size / 512).min(65_535 * 16 * 255);
    if total_sectors == 0 {
        return (0, 0, 0);
    }
    let (heads, spt) = if total_sectors >= (65_535 * 16 * 63) as u64 {
        (16u8, 255u8)
    } else {
        (16u8, 63u8)
    };
    let cylinders = (total_sectors / u64::from(heads) / u64::from(spt)) as u16;
    (cylinders, heads, spt)
}

fn put_u16_be(buf: &mut [u8], offset: usize, value: u16) {
    buf[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn put_u32_be(buf: &mut [u8], offset: usize, value: u32) {
    buf[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn put_u64_be(buf: &mut [u8], offset: usize, value: u64) {
    buf[offset..offset + 8].copy_from_slice(&value.to_be_bytes());
}
