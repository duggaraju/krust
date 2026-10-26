use clap::{Parser, Subcommand, ValueEnum};
use ovmf_prebuilt::{Arch, FileType, Prebuilt, Source};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

mod disk;

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
    /// Create a FAT32 VHD image from a directory
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

    #[arg(long, value_name = "PATH")]
    vhd: Option<PathBuf>,

    #[arg(long = "no-vhd")]
    no_vhd: bool,

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
    /// FAT32 filesystem (QEMU VHD, default)
    Fat,
    /// ext4 filesystem — requires mkfs.ext4 on the host
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

    /// Image size in MiB (minimum 32)
    #[arg(long, default_value = "64")]
    size_mib: u64,

    /// Comma-separated list of directory names to exclude
    #[arg(long, default_value = "")]
    exclude: String,

    /// Filesystem format to use
    #[arg(long, value_enum, default_value = "fat")]
    format: DiskFormat,
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
        DiskFormat::Fat => "FAT32",
        DiskFormat::Ext4 => "ext4",
    };
    eprintln!(
        "creating {}MiB {} VHD from '{}' → '{}' (exclude: {:?})",
        args.size_mib,
        format_name,
        args.source_dir.display(),
        args.output.display(),
        exclude_dirs
    );
    let result = match args.format {
        DiskFormat::Fat => disk::create_fat_vhd_from_dir(
            &args.source_dir,
            &args.output,
            args.size_mib,
            exclude_dirs,
        ),
        DiskFormat::Ext4 => disk::create_ext4_vhd_from_dir(
            &args.source_dir,
            &args.output,
            args.size_mib,
            exclude_dirs,
        ),
    };
    result.unwrap_or_else(|err| {
        eprintln!("error: {}", err);
        std::process::exit(1);
    });
    eprintln!("done: {}", args.output.display());
}

fn cmd_run(cli: RunArgs) -> Result<(), String> {
    // read env variables that were set in build script
    let kernel_path = env!("KERNEL_PATH");
    let kernel_bios_path = env!("KERNEL_BIOS_PATH");
    let boot_log_level = env!("KRUST_BOOT_LOG_LEVEL");
    let boot_syscall_trace = env!("KRUST_BOOT_SYSCALL_TRACE");
    let uefi_path = env!("UEFI_PATH");
    let bios_path = env!("BIOS_PATH");

    let mode_uefi = matches!(cli.mode, BootMode::Uefi);
    let uefi = mode_uefi;
    let shell_port = serial_shell_port(&cli);

    let sata_vhd = if cli.no_vhd {
        None
    } else if let Some(path) = cli.vhd {
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

    if uefi {
        let prebuilt =
            Prebuilt::fetch(Source::LATEST, "target/ovmf").expect("failed to update prebuilt");

        let code = prebuilt.get_file(Arch::X64, FileType::Code);
        let vars = prebuilt.get_file(Arch::X64, FileType::Vars);
        let image_path = if let Some(shell_port) = shell_port {
            build_headless_uefi_image(kernel_path, boot_log_level, shell_port, boot_syscall_trace)
        } else {
            PathBuf::from(uefi_path)
        };

        cmd.arg("-drive")
            .arg(format!("format=raw,file={}", image_path.display()));
        cmd.arg("-drive").arg(format!(
            "if=pflash,format=raw,unit=0,file={},readonly=on",
            code.display()
        ));
        // copy vars and enable rw instead of snapshot if you want to store data (e.g. enroll secure boot keys)
        cmd.arg("-drive").arg(format!(
            "if=pflash,format=raw,unit=1,file={},snapshot=on",
            vars.display()
        ));
    } else {
        let image_path = if let Some(shell_port) = shell_port {
            build_headless_bios_image(
                kernel_bios_path,
                boot_log_level,
                shell_port,
                boot_syscall_trace,
            )
        } else {
            PathBuf::from(bios_path)
        };
        cmd.arg("-drive")
            .arg(format!("format=raw,file={}", image_path.display()));
    }

    if !cli.qemu_args.is_empty() {
        eprintln!("extra qemu args: {:?}", cli.qemu_args);
        cmd.args(&cli.qemu_args);
    }

    run_qemu(&mut cmd)
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

fn build_headless_uefi_image(
    kernel_path: &str,
    boot_log_level: &str,
    shell_port: u8,
    boot_syscall_trace: &str,
) -> PathBuf {
    let temp_dir = env::temp_dir();
    let pid = std::process::id();
    let ramdisk_path = temp_dir.join(format!("krust-headless-{pid}.ramdisk"));
    let image_path = temp_dir.join(format!("krust-headless-{pid}.img"));

    fs::write(
        &ramdisk_path,
        headless_boot_ramdisk(boot_log_level, shell_port, boot_syscall_trace),
    )
    .expect("failed to create headless ramdisk");

    bootloader::UefiBoot::new(PathBuf::from(kernel_path).as_path())
        .set_ramdisk(&ramdisk_path)
        .create_disk_image(&image_path)
        .expect("failed to create headless uefi image");

    image_path
}

fn build_headless_bios_image(
    kernel_path: &str,
    boot_log_level: &str,
    shell_port: u8,
    boot_syscall_trace: &str,
) -> PathBuf {
    let temp_dir = env::temp_dir();
    let pid = std::process::id();
    let ramdisk_path = temp_dir.join(format!("krust-headless-{pid}.ramdisk"));
    let image_path = temp_dir.join(format!("krust-headless-{pid}.bios.img"));

    fs::write(
        &ramdisk_path,
        headless_boot_ramdisk(boot_log_level, shell_port, boot_syscall_trace),
    )
    .expect("failed to create headless ramdisk");

    bootloader::BiosBoot::new(PathBuf::from(kernel_path).as_path())
        .set_ramdisk(&ramdisk_path)
        .create_disk_image(&image_path)
        .expect("failed to create headless bios image");

    image_path
}

fn headless_boot_ramdisk(log_level: &str, shell_port: u8, syscall_trace: &str) -> Vec<u8> {
    let boot_toml = format!(
        "log_level = \"{log_level}\"\nshell_port = {shell_port}\nshell_console = \"serial\"\nsyscall_trace = {syscall_trace}\n"
    );
    make_cpio_newc(&[("boot.toml", boot_toml.as_bytes())])
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
        let ramdisk = headless_boot_ramdisk("debug", port, trace);
        assert_eq!(&ramdisk[..6], b"070701");
        let hex = |range: std::ops::Range<usize>| {
            usize::from_str_radix(std::str::from_utf8(&ramdisk[range]).unwrap(), 16).unwrap()
        };
        let name_len = hex(94..102);
        let data_len = hex(54..62);
        assert_eq!(&ramdisk[110..110 + name_len], b"boot.toml\0");
        let data_start = (110 + name_len + 3) & !3;
        let config = std::str::from_utf8(&ramdisk[data_start..data_start + data_len]).unwrap();
        assert_eq!(
            config,
            format!(
                "log_level = \"debug\"\nshell_port = {port}\nshell_console = \"serial\"\nsyscall_trace = {trace}\n"
            )
        );
        let trailer_start = (data_start + data_len + 3) & !3;
        assert_eq!(&ramdisk[trailer_start..trailer_start + 6], b"070701");
        assert_eq!(
            &ramdisk[trailer_start + 110..trailer_start + 121],
            b"TRAILER!!!\0"
        );
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
