use clap::{Parser, Subcommand, ValueEnum};
use ovmf_prebuilt::{Arch, FileType, Prebuilt, Source};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

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

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Run(args) => cmd_run(args),
        Commands::Mkdisk(args) => cmd_mkdisk(args),
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

fn cmd_run(cli: RunArgs) {
    // read env variables that were set in build script
    let kernel_path = env!("KERNEL_PATH");
    let kernel_bios_path = env!("KERNEL_BIOS_PATH");
    let boot_log_level = env!("KRUST_BOOT_LOG_LEVEL");
    let uefi_path = env!("UEFI_PATH");
    let bios_path = env!("BIOS_PATH");

    let mode_uefi = matches!(cli.mode, BootMode::Uefi);
    let uefi = mode_uefi;
    let use_secondary_serial = cli.headless || cli.pty;
    let serial_shell_boot = use_secondary_serial || (!uefi && qemu_display_none(&cli.qemu_args));

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
        let image_path = if serial_shell_boot {
            build_headless_uefi_image(
                kernel_path,
                boot_log_level,
                if use_secondary_serial { 2 } else { 1 },
            )
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
        let image_path = if serial_shell_boot {
            build_headless_bios_image(
                kernel_bios_path,
                boot_log_level,
                if use_secondary_serial { 2 } else { 1 },
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

    let mut child = cmd.spawn().expect("failed to start qemu-system-x86_64");
    let status = child.wait().expect("failed to wait on qemu");
    match status.code().unwrap_or(1) {
        0x10 => 0, // success
        0x11 => 1, // failure
        _ => 2,    // unknown fault
    };
}

fn build_headless_uefi_image(kernel_path: &str, boot_log_level: &str, shell_port: u8) -> PathBuf {
    let temp_dir = env::temp_dir();
    let pid = std::process::id();
    let ramdisk_path = temp_dir.join(format!("krust-headless-{pid}.ramdisk"));
    let image_path = temp_dir.join(format!("krust-headless-{pid}.img"));

    let boot_toml = format!(
        "log_level = \"{}\"\nshell_port = {}\nshell_console = \"serial\"\n",
        boot_log_level, shell_port
    );
    fs::write(
        &ramdisk_path,
        make_cpio_newc(&[("boot.toml", boot_toml.as_bytes())]),
    )
    .expect("failed to create headless ramdisk");

    bootloader::UefiBoot::new(PathBuf::from(kernel_path).as_path())
        .set_ramdisk(&ramdisk_path)
        .create_disk_image(&image_path)
        .expect("failed to create headless uefi image");

    image_path
}

fn build_headless_bios_image(kernel_path: &str, boot_log_level: &str, shell_port: u8) -> PathBuf {
    let temp_dir = env::temp_dir();
    let pid = std::process::id();
    let ramdisk_path = temp_dir.join(format!("krust-headless-{pid}.ramdisk"));
    let image_path = temp_dir.join(format!("krust-headless-{pid}.bios.img"));

    let boot_toml = format!(
        "log_level = \"{}\"\nshell_port = {}\nshell_console = \"serial\"\n",
        boot_log_level, shell_port
    );
    fs::write(
        &ramdisk_path,
        make_cpio_newc(&[("boot.toml", boot_toml.as_bytes())]),
    )
    .expect("failed to create headless ramdisk");

    bootloader::BiosBoot::new(PathBuf::from(kernel_path).as_path())
        .set_ramdisk(&ramdisk_path)
        .create_disk_image(&image_path)
        .expect("failed to create headless bios image");

    image_path
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
