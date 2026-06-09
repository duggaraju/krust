use ovmf_prebuilt::{Arch, FileType, Prebuilt, Source};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, exit};

fn main() {
    // read env variables that were set in build script
    let kernel_path = env!("KERNEL_PATH");
    let boot_log_level = env!("KRUST_BOOT_LOG_LEVEL");
    let uefi_path = env!("UEFI_PATH");
    let bios_path = env!("BIOS_PATH");

    // parse mode from CLI
    let args: Vec<String> = env::args().collect();
    let prog = &args[0];

    let mut uefi = None;
    let mut headless = false;
    let mut pty = false;
    for arg in args.iter().skip(1) {
        match arg.to_lowercase().as_str() {
            "uefi" => uefi = Some(true),
            "bios" => uefi = Some(false),
            "--headless" | "headless" => headless = true,
            "--pty" | "pty" => pty = true,
            "-h" | "--help" => {
                println!("Usage: {prog} [uefi|bios] [--headless|--pty]");
                println!("  uefi       - boot using OVMF (UEFI)");
                println!("  bios       - boot using legacy BIOS");
                println!("  --headless - run without a QEMU window (UEFI only)");
                println!("  --pty      - use COM2 PTY for trace output");
                exit(0);
            }
            _ => {
                eprintln!("Usage: {prog} [uefi|bios] [--headless|--pty]");
                exit(1);
            }
        }
    }

    let uefi = uefi.unwrap_or_else(|| {
        eprintln!("Usage: {prog} [uefi|bios] [--headless|--pty]");
        exit(1);
    });
    let uefi = if headless || pty { true } else { uefi };

    let mut cmd = Command::new("qemu-system-x86_64");
    if headless {
        let trace_path = env::temp_dir().join(format!("krust-trace-{}.log", std::process::id()));
        eprintln!("headless kernel trace log: {}", trace_path.display());
        cmd.arg("-display").arg("none");
        cmd.arg("-monitor").arg("none");
        cmd.arg("-chardev")
            .arg(format!("file,path={},id=trace", trace_path.display()));
        cmd.arg("-serial").arg("chardev:trace");
        cmd.arg("-serial").arg("stdio");
    } else if pty {
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

    if uefi {
        let prebuilt =
            Prebuilt::fetch(Source::LATEST, "target/ovmf").expect("failed to update prebuilt");

        let code = prebuilt.get_file(Arch::X64, FileType::Code);
        let vars = prebuilt.get_file(Arch::X64, FileType::Vars);
        let image_path = if headless || pty {
            build_headless_uefi_image(kernel_path, boot_log_level)
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
        cmd.arg("-drive")
            .arg(format!("format=raw,file={bios_path}"));
    }

    let mut child = cmd.spawn().expect("failed to start qemu-system-x86_64");
    let status = child.wait().expect("failed to wait on qemu");
    match status.code().unwrap_or(1) {
        0x10 => 0, // success
        0x11 => 1, // failure
        _ => 2,    // unknown fault
    };
}

fn build_headless_uefi_image(kernel_path: &str, boot_log_level: &str) -> PathBuf {
    let temp_dir = env::temp_dir();
    let pid = std::process::id();
    let ramdisk_path = temp_dir.join(format!("krust-headless-{pid}.ramdisk"));
    let image_path = temp_dir.join(format!("krust-headless-{pid}.img"));

    let boot_toml = format!(
        "log_level = \"{}\"\nshell_port = 2\nshell_console = \"serial\"\n",
        boot_log_level
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
