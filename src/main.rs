use clap::{Parser, Subcommand, ValueEnum};
use ovmf_prebuilt::{Arch, FileType, Prebuilt, Source};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

#[path = "../boot_settings.rs"]
mod boot_settings;
mod disk;
use boot_settings::BootSettings;

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum BootMode {
    Uefi,
    Bios,
}

#[derive(Parser, Debug)]
#[command(name = "krust", about = "krust kernel tools")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run the kernel in QEMU
    Run(RunArgs),
    /// Create a FAT or ext4 VHD image from a directory
    Mkdisk(MkdiskArgs),
}

#[derive(Parser, Debug)]
struct RunArgs {
    #[arg(value_enum, default_value_t = BootMode::Uefi)]
    mode: BootMode,

    #[arg(long)]
    headless: bool,

    #[arg(long)]
    pty: bool,

    #[arg(long, value_name = "PATH", conflicts_with = "no_vhd")]
    vhd: Option<PathBuf>,

    #[arg(long = "no-vhd")]
    no_vhd: bool,

    /// Boot TOML settings; overrides the VHD's automatic .boot.toml sidecar.
    #[arg(long, value_name = "PATH")]
    boot_config: Option<PathBuf>,

    /// Extra raw arguments passed directly to qemu-system-x86_64.
    /// Usage: krust run <mode> [options] -- <qemu args...>
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "QEMU_ARGS"
    )]
    qemu_args: Vec<String>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum DiskFormat {
    /// FAT filesystem (automatically selected FAT12/16/32)
    Fat,
    /// ext4 filesystem — requires mkfs.ext4 and debugfs on the host
    Ext4,
}

#[derive(Parser, Debug)]
struct MkdiskArgs {
    /// Source directory to pack into the image
    #[arg(value_name = "DIR")]
    source_dir: PathBuf,

    /// Output VHD path
    #[arg(value_name = "OUTPUT")]
    output: PathBuf,

    /// Image data size in MiB (minimum: FAT 4, ext4 16)
    #[arg(long, default_value = "64")]
    size_mib: u64,

    /// Comma-separated list of directory names to exclude
    #[arg(long, default_value = "")]
    exclude: String,

    /// Filesystem format to use
    #[arg(long, value_enum, default_value = "fat")]
    format: DiskFormat,

    /// Use this fstab instead of DIR/etc/fstab or generated defaults.
    #[arg(long, value_name = "PATH")]
    fstab: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Commands::Run(args) => cmd_run(args),
        Commands::Mkdisk(args) => {
            cmd_mkdisk(args);
            Ok(())
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_mkdisk(args: MkdiskArgs) {
    if !args.source_dir.is_dir() {
        eprintln!("error: '{}' is not a directory", args.source_dir.display());
        std::process::exit(1);
    }
    let exclude_dirs: std::collections::HashSet<_> = args
        .exclude
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    let format_name = match args.format {
        DiskFormat::Fat => "FAT",
        DiskFormat::Ext4 => "ext4",
    };
    let custom_fstab = args
        .fstab
        .as_ref()
        .map(fs::read_to_string)
        .transpose()
        .unwrap_or_else(|e| {
            eprintln!("error: cannot read fstab: {e}");
            std::process::exit(1);
        });
    let config = disk::DiskMountConfig::load(
        &args.source_dir,
        match args.format {
            DiskFormat::Fat => "fat",
            DiskFormat::Ext4 => "ext4",
        },
        custom_fstab.as_deref(),
    )
    .unwrap_or_else(|e| {
        eprintln!("error: invalid mount configuration: {e}");
        std::process::exit(1);
    });
    eprintln!(
        "creating {}MiB {} VHD from '{}' → '{}' (exclude: {:?})",
        args.size_mib,
        format_name,
        args.source_dir.display(),
        args.output.display(),
        exclude_dirs
    );
    let result = match args.format {
        DiskFormat::Fat => disk::create_fat_vhd_with_config(
            &args.source_dir,
            &args.output,
            args.size_mib,
            exclude_dirs,
            &config,
        ),
        DiskFormat::Ext4 => disk::create_ext4_vhd_with_config(
            &args.source_dir,
            &args.output,
            args.size_mib,
            exclude_dirs,
            &config,
        ),
    };
    result.unwrap_or_else(|err| {
        eprintln!("error: {}", err);
        std::process::exit(1);
    });
    let sidecar = boot_sidecar_path(&args.output);
    fs::write(
        &sidecar,
        format!(
            "root_fs = \"{}\"\nroot_device = \"{}\"\n",
            config.settings.root_fs, config.settings.root_device
        ),
    )
    .unwrap_or_else(|e| {
        eprintln!("error: cannot write {}: {e}", sidecar.display());
        std::process::exit(1);
    });
    eprintln!("boot configuration: {}", sidecar.display());
    eprintln!("done: {}", args.output.display());
}

fn cmd_run(cli: RunArgs) -> Result<(), String> {
    // read env variables that were set in build script
    let kernel_path = env!("KERNEL_PATH");
    let kernel_bios_path = env!("KERNEL_BIOS_PATH");

    let mode_uefi = matches!(cli.mode, BootMode::Uefi);
    let uefi = mode_uefi;
    let shell_port = serial_shell_port(&cli);

    let sata_vhd = if cli.no_vhd {
        None
    } else if let Some(path) = cli.vhd.clone() {
        if !path.exists() {
            eprintln!("error: VHD not found: {}", path.display());
            eprintln!(
                "hint:  cargo run -Z bindeps -- mkdisk <rootfs_dir> {} [--format fat|ext4]",
                path.display()
            );
            std::process::exit(1);
        }
        Some(path)
    } else {
        None
    };
    let mut settings = load_boot_settings(&cli, sata_vhd.as_deref())?;
    if let Some(port) = shell_port {
        settings.shell_port = port;
        settings.shell_console = "serial".into();
    }
    let mut cmd = Command::new("qemu-system-x86_64");
    if cli.headless {
        let trace_path = env::temp_dir().join(format!("krust-trace-{}.log", std::process::id()));
        eprintln!("headless kernel trace log: {}", trace_path.display());
        cmd.arg("-display").arg("none");
        cmd.arg("-monitor").arg("none");
        cmd.arg("-chardev")
            .arg(format!("file,path={},id=trace", trace_path.display()));
        cmd.arg("-serial").arg("chardev:trace");
        cmd.arg("-serial").arg("stdio");
    } else if cli.pty {
        cmd.arg("-display").arg("none");
        cmd.arg("-monitor").arg("none");
        cmd.arg("-serial").arg("stdio");
        cmd.arg("-chardev").arg("pty,id=trace");
        cmd.arg("-serial").arg("chardev:trace");
    } else {
        cmd.arg("-serial").arg("stdio");
        cmd.arg("-monitor").arg("none");
        cmd.arg("-display").arg("default");
    }
    // enable the guest to exit qemu
    cmd.arg("-device")
        .arg("isa-debug-exit,iobase=0xf4,iosize=0x04");

    if let Some(path) = sata_vhd.as_ref() {
        cmd.arg("-device").arg("ahci,id=ahci0");
        cmd.arg("-drive").arg(format!(
            "if=none,id=krust_disk,format=vpc,file={}",
            path.display()
        ));
        cmd.arg("-device")
            .arg("ide-hd,drive=krust_disk,bus=ahci0.0");
        eprintln!("attached SATA VHD: {}", path.display());
    }

    let boot_image = build_boot_image(
        if uefi { kernel_path } else { kernel_bios_path },
        &settings,
        uefi,
    )?;
    cmd.arg("-drive").arg(format!(
        "format=raw,file={}",
        boot_image.directory.join("boot.img").display()
    ));
    if uefi {
        let prebuilt =
            Prebuilt::fetch(Source::LATEST, "target/ovmf").expect("failed to update prebuilt");

        let code = prebuilt.get_file(Arch::X64, FileType::Code);
        let vars = prebuilt.get_file(Arch::X64, FileType::Vars);
        cmd.arg("-drive").arg(format!(
            "if=pflash,format=raw,unit=0,file={},readonly=on",
            code.display()
        ));
        // copy vars and enable rw instead of snapshot if you want to store data (e.g. enroll secure boot keys)
        cmd.arg("-drive").arg(format!(
            "if=pflash,format=raw,unit=1,file={},snapshot=on",
            vars.display()
        ));
    }

    if !cli.qemu_args.is_empty() {
        eprintln!("extra qemu args: {:?}", cli.qemu_args);
        cmd.args(&cli.qemu_args);
    }

    run_qemu(&mut cmd)
}

fn boot_sidecar_path(image: &std::path::Path) -> PathBuf {
    let mut name = image.as_os_str().to_owned();
    name.push(".boot.toml");
    PathBuf::from(name)
}

fn load_boot_settings(
    cli: &RunArgs,
    disk: Option<&std::path::Path>,
) -> Result<BootSettings, String> {
    let defaults = BootSettings::parse(include_str!(env!("BOOT_CONFIG_PATH")))?;
    let path = cli
        .boot_config
        .clone()
        .or_else(|| disk.map(boot_sidecar_path));
    if let Some(path) = path {
        match fs::read_to_string(&path) {
            Ok(contents) => {
                return BootSettings::parse_with_defaults(&contents, defaults)
                    .map_err(|e| format!("{}: {e}", path.display()));
            }
            Err(e) if cli.boot_config.is_none() && e.kind() == std::io::ErrorKind::NotFound => {
                eprintln!(
                    "no boot sidecar at {}; using legacy ext4 root defaults",
                    path.display()
                );
            }
            Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
        }
    }
    if disk.is_none() {
        Ok(BootSettings {
            root_fs: "ramfs".into(),
            root_device: "none".into(),
            ..defaults
        })
    } else {
        Ok(defaults)
    }
}

fn run_qemu(cmd: &mut Command) -> Result<(), String> {
    let mut child = cmd
        .spawn()
        .map_err(|err| format!("failed to start qemu-system-x86_64: {err}"))?;
    let status = child
        .wait()
        .map_err(|err| format!("failed to wait on qemu-system-x86_64: {err}"))?;
    check_qemu_exit(status.code())
}

fn check_qemu_exit(code: Option<i32>) -> Result<(), String> {
    // isa-debug-exit encodes guest values as (value << 1) | 1.
    match code {
        Some(33) => Ok(()),
        Some(35) => Err("QEMU reported guest failure (guest 0x11, raw exit status 35)".into()),
        Some(code) => Err(format!(
            "QEMU terminated unexpectedly (raw exit status {code}); expected 33 (guest success 0x10)"
        )),
        None => Err("QEMU terminated without an exit code (possibly killed by a signal)".into()),
    }
}

struct TemporaryBootImage {
    directory: PathBuf,
}

impl Drop for TemporaryBootImage {
    fn drop(&mut self) {
        for name in ["boot.img", "boot.ramdisk"] {
            let path = self.directory.join(name);
            if let Err(error) = fs::remove_file(&path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    eprintln!("cannot remove {}: {error}", path.display());
                }
            }
        }
        if let Err(error) = fs::remove_dir(&self.directory) {
            eprintln!("cannot remove {}: {error}", self.directory.display());
        }
    }
}

fn build_boot_image(
    kernel_path: &str,
    settings: &BootSettings,
    uefi: bool,
) -> Result<TemporaryBootImage, String> {
    let directory = env::temp_dir().join(format!("krust-boot-{}", std::process::id()));
    fs::create_dir(&directory)
        .map_err(|e| format!("cannot create {}: {e}", directory.display()))?;
    let image = TemporaryBootImage { directory };
    let ramdisk = image.directory.join("boot.ramdisk");
    let output = image.directory.join("boot.img");
    fs::write(&ramdisk, boot_ramdisk(settings))
        .map_err(|e| format!("cannot write boot ramdisk: {e}"))?;
    let result = if uefi {
        bootloader::UefiBoot::new(std::path::Path::new(kernel_path))
            .set_ramdisk(&ramdisk)
            .create_disk_image(&output)
    } else {
        bootloader::BiosBoot::new(std::path::Path::new(kernel_path))
            .set_ramdisk(&ramdisk)
            .create_disk_image(&output)
    };
    result.map_err(|e| format!("cannot create boot image: {e}"))?;
    Ok(image)
}

fn boot_ramdisk(settings: &BootSettings) -> Vec<u8> {
    make_cpio_newc(&[
        ("boot.toml", settings.to_toml().as_bytes()),
        ("etc/fstab", settings.default_fstab().as_bytes()),
    ])
}

fn serial_shell_port(cli: &RunArgs) -> Option<u8> {
    if cli.headless || cli.pty {
        Some(2)
    } else if cli.mode == BootMode::Bios && qemu_display_none(&cli.qemu_args) {
        Some(1)
    } else {
        None
    }
}

fn qemu_display_none(qemu_args: &[String]) -> bool {
    qemu_args
        .windows(2)
        .any(|w| (w[0] == "--display" || w[0] == "-display") && w[1].eq_ignore_ascii_case("none"))
}

fn make_cpio_newc(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut ino = 1u32;

    for (name, data) in entries {
        append_cpio_entry(&mut out, ino, name, data);
        ino += 1;
    }

    append_cpio_entry(&mut out, ino, "TRAILER!!!", &[]);
    out
}

fn append_cpio_entry(out: &mut Vec<u8>, ino: u32, name: &str, data: &[u8]) {
    fn push_hex(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(format!("{:08x}", value).as_bytes());
    }

    fn pad4(out: &mut Vec<u8>) {
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }

    out.extend_from_slice(b"070701");
    push_hex(out, ino);
    push_hex(out, 0o100644);
    push_hex(out, 0);
    push_hex(out, 0);
    push_hex(out, 1);
    push_hex(out, 0);
    push_hex(out, data.len() as u32);
    push_hex(out, 0);
    push_hex(out, 0);
    push_hex(out, 0);
    push_hex(out, 0);
    push_hex(out, (name.len() + 1) as u32);
    push_hex(out, 0);

    out.extend_from_slice(name.as_bytes());
    out.push(0);
    pad4(out);
    out.extend_from_slice(data);
    pad4(out);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_boot_config_preserves_trace_and_ports() {
        for mode in ["bios", "uefi"] {
            for trace in ["true", "false"] {
                for (options, expected_port) in [
                    (vec!["--headless"], 2),
                    (vec!["--pty"], 2),
                    (vec!["--headless", "--pty"], 2),
                ] {
                    let cli = Cli::parse_from(["krust", "run", mode].into_iter().chain(options));
                    let Commands::Run(args) = cli.command else {
                        unreachable!()
                    };
                    let port = serial_shell_port(&args).unwrap();
                    assert_eq!(port, expected_port);
                    assert_boot_ramdisk(port, trace);
                }
            }
        }
    }

    #[test]
    fn boot_config_preserves_display_none_and_ordinary_boot() {
        for mode in ["bios", "uefi"] {
            let cli = Cli::parse_from(["krust", "run", mode]);
            let Commands::Run(args) = cli.command else {
                unreachable!()
            };
            assert_eq!(serial_shell_port(&args), None);

            for display_arg in ["-display", "--display"] {
                let cli = Cli::parse_from(["krust", "run", mode, "--", display_arg, "none"]);
                let Commands::Run(args) = cli.command else {
                    unreachable!()
                };
                let port = serial_shell_port(&args);
                assert_eq!(port, if mode == "bios" { Some(1) } else { None });
                if let Some(port) = port {
                    for trace in ["true", "false"] {
                        assert_boot_ramdisk(port, trace);
                    }
                }
            }
        }
    }

    fn assert_boot_ramdisk(port: u8, trace: &str) {
        let settings = BootSettings {
            log_level: "debug".into(),
            shell_port: port,
            shell_console: "serial".into(),
            syscall_trace: trace == "true",
            ..Default::default()
        };
        let ramdisk = boot_ramdisk(&settings);
        assert_eq!(&ramdisk[..6], b"070701");
        let hex = |range: std::ops::Range<usize>| {
            usize::from_str_radix(std::str::from_utf8(&ramdisk[range]).unwrap(), 16).unwrap()
        };
        let name_len = hex(94..102);
        let data_len = hex(54..62);
        assert_eq!(&ramdisk[110..110 + name_len], b"boot.toml\0");
        let data_start = (110 + name_len + 3) & !3;
        let config = std::str::from_utf8(&ramdisk[data_start..data_start + data_len]).unwrap();
        assert_eq!(BootSettings::parse(config).unwrap(), settings);
        let fstab_start = (data_start + data_len + 3) & !3;
        let fstab_name_len = hex(fstab_start + 94..fstab_start + 102);
        let fstab_len = hex(fstab_start + 54..fstab_start + 62);
        assert_eq!(
            &ramdisk[fstab_start + 110..fstab_start + 110 + fstab_name_len],
            b"etc/fstab\0"
        );
        let fstab_data = (fstab_start + 110 + fstab_name_len + 3) & !3;
        assert_eq!(
            &ramdisk[fstab_data..fstab_data + fstab_len],
            settings.default_fstab().as_bytes()
        );
        let trailer_start = (fstab_data + fstab_len + 3) & !3;
        assert_eq!(&ramdisk[trailer_start..trailer_start + 6], b"070701");
        assert_eq!(
            &ramdisk[trailer_start + 110..trailer_start + 121],
            b"TRAILER!!!\0"
        );
    }

    #[test]
    fn root_settings_survive_all_console_modes_and_firmware() {
        for mode in ["bios", "uefi"] {
            for options in [vec![], vec!["--headless"], vec!["--pty"]] {
                let cli = Cli::parse_from(["krust", "run", mode].into_iter().chain(options));
                let Commands::Run(args) = cli.command else {
                    unreachable!()
                };
                let mut settings =
                    BootSettings::parse("root_fs = \"fat\"\nroot_device = \"/dev/sdb\"").unwrap();
                if let Some(port) = serial_shell_port(&args) {
                    settings.shell_port = port;
                    settings.shell_console = "serial".into();
                }
                let ramdisk = boot_ramdisk(&settings);
                assert!(
                    ramdisk
                        .windows(b"root_fs = \"fat\"".len())
                        .any(|window| window == b"root_fs = \"fat\"")
                );
                assert!(
                    ramdisk
                        .windows(b"/dev/sdb / fat".len())
                        .any(|window| window == b"/dev/sdb / fat")
                );
            }
        }
        let cli = Cli::parse_from(["krust", "run", "bios", "--no-vhd"]);
        let Commands::Run(args) = cli.command else {
            unreachable!()
        };
        let settings = load_boot_settings(&args, None).unwrap();
        assert_eq!(settings.root_fs, "ramfs");
        assert_eq!(settings.root_device, "none");
    }

    #[test]
    fn boot_sidecar_selection_and_explicit_override() {
        let directory = env::temp_dir().join(format!("krust-settings-test-{}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let disk = directory.join("test.vhd");
        let sidecar = boot_sidecar_path(&disk);
        let custom = directory.join("custom.toml");
        fs::write(&sidecar, "root_fs = \"fat\"\n").unwrap();
        fs::write(&custom, "root_fs = \"ramfs\"\nroot_device = \"none\"\n").unwrap();
        let cli = Cli::parse_from(["krust", "run", "bios"]);
        let Commands::Run(args) = cli.command else {
            unreachable!()
        };
        assert_eq!(
            load_boot_settings(&args, Some(&disk)).unwrap().root_fs,
            "fat"
        );
        let cli = Cli::parse_from([
            "krust",
            "run",
            "bios",
            "--boot-config",
            custom.to_str().unwrap(),
        ]);
        let Commands::Run(args) = cli.command else {
            unreachable!()
        };
        assert_eq!(
            load_boot_settings(&args, Some(&disk)).unwrap().root_fs,
            "ramfs"
        );
        fs::write(&custom, "root_fs = \"bad\"\n").unwrap();
        assert!(load_boot_settings(&args, Some(&disk)).is_err());
        fs::remove_file(&custom).unwrap();
        assert!(load_boot_settings(&args, Some(&disk)).is_err());
        fs::remove_file(&sidecar).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn qemu_exit_guest_success() {
        assert_eq!(check_qemu_exit(Some(33)), Ok(()));
    }

    #[test]
    fn qemu_exit_guest_failure() {
        let err = check_qemu_exit(Some(35)).unwrap_err();
        assert!(err.contains("guest failure"));
        assert!(err.contains("0x11"));
        assert!(err.contains("35"));
    }

    #[test]
    fn qemu_exit_zero_is_not_guest_success() {
        let err = check_qemu_exit(Some(0)).unwrap_err();
        assert!(err.contains("unexpectedly"));
        assert!(err.contains("raw exit status 0"));
    }

    #[test]
    fn qemu_exit_unexpected_status() {
        for code in [1, 16, 17, 42] {
            let err = check_qemu_exit(Some(code)).unwrap_err();
            assert!(err.contains("unexpectedly"));
            assert!(err.contains(&format!("raw exit status {code}")));
        }
    }

    #[test]
    fn qemu_exit_no_code() {
        let err = check_qemu_exit(None).unwrap_err();
        assert!(err.contains("without an exit code"));
        assert!(err.contains("signal"));
    }

    #[test]
    fn qemu_launch_error() {
        let impossible_path = env::current_exe().unwrap().join("qemu-system-x86_64");
        let err = run_qemu(&mut Command::new(impossible_path)).unwrap_err();
        assert!(err.contains("failed to start qemu-system-x86_64"));
    }
}
