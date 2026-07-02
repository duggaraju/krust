use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::vec::Vec;

fn main() {
    println!("cargo:rerun-if-env-changed=KRUST_BOOT_LOG_LEVEL");
    println!("cargo:rerun-if-env-changed=KRUST_BOOT_SHELL_CONSOLE");
    println!("cargo:rerun-if-env-changed=KRUST_BOOT_SYSCALL_TRACE");

    // set by cargo, build scripts should use this directory for output files
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    // set by cargo's artifact dependency feature, see
    // https://doc.rust-lang.org/nightly/cargo/reference/unstable.html#artifact-dependencies
    let kernel = PathBuf::from(std::env::var_os("CARGO_BIN_FILE_KERNEL_kernel").unwrap());
    let boot_log_level = std::env::var("KRUST_BOOT_LOG_LEVEL").unwrap_or_else(|_| "info".into());
    let boot_shell_console =
        std::env::var("KRUST_BOOT_SHELL_CONSOLE").unwrap_or_else(|_| "auto".into());
    let boot_syscall_trace =
        std::env::var("KRUST_BOOT_SYSCALL_TRACE").unwrap_or_else(|_| "false".into());
    let boot_toml = format!(
        "log_level = \"{}\"\nshell_port = 1\nshell_console = \"{}\"\nsyscall_trace = {}\n",
        boot_log_level, boot_shell_console, boot_syscall_trace
    );
    let ramdisk_path = out_dir.join("boot.ramdisk");
    write_boot_ramdisk(&ramdisk_path, &boot_toml);

    // create an UEFI disk image (optional)
    let uefi_path = out_dir.join("uefi.img");
    bootloader::UefiBoot::new(&kernel)
        .set_ramdisk(&ramdisk_path)
        .create_disk_image(&uefi_path)
        .unwrap();

    // Strip debug info for the BIOS image: the BIOS bootloader loads the full ELF
    // over slow INT 13h disk I/O. A debug build can be 15MB+ which takes minutes;
    // stripping reduces it to ~3MB and brings boot time under a few seconds.
    let kernel_for_bios = strip_kernel_for_bios(&kernel, &out_dir);

    // create a BIOS disk image
    let bios_path = out_dir.join("bios.img");
    bootloader::BiosBoot::new(&kernel_for_bios)
        .set_ramdisk(&ramdisk_path)
        .create_disk_image(&bios_path)
        .unwrap();

    // pass the disk image paths as env variables to the `main.rs`
    println!("cargo:rustc-env=KERNEL_PATH={}", kernel.display());
    println!(
        "cargo:rustc-env=KERNEL_BIOS_PATH={}",
        kernel_for_bios.display()
    );
    println!("cargo:rustc-env=KRUST_BOOT_LOG_LEVEL={}", boot_log_level);
    println!("cargo:rustc-env=UEFI_PATH={}", uefi_path.display());
    println!("cargo:rustc-env=BIOS_PATH={}", bios_path.display());
}

fn strip_kernel_for_bios(kernel: &PathBuf, out_dir: &PathBuf) -> PathBuf {
    let stripped = out_dir.join("kernel-bios-stripped");
    // Try llvm-objcopy first (available via llvm-tools-preview rustup component),
    // fall back to system strip if not found.
    let stripped_ok = try_llvm_objcopy(kernel, &stripped)
        || try_system_strip(kernel, &stripped);
    if stripped_ok {
        stripped
    } else {
        // Strip failed — use the original (slow but functional).
        eprintln!("cargo:warning=Could not strip kernel debug info for BIOS image; BIOS boot will be slow");
        kernel.clone()
    }
}

fn try_llvm_objcopy(src: &PathBuf, dst: &PathBuf) -> bool {
    // Find llvm-objcopy via llvm-tools or on PATH.
    let candidates = ["llvm-objcopy", "llvm-objcopy-18", "llvm-objcopy-17", "llvm-objcopy-16"];
    for tool in &candidates {
        let status = Command::new(tool)
            .args(["--strip-debug", "--strip-unneeded"])
            .arg(src)
            .arg(dst)
            .status();
        if let Ok(s) = status {
            if s.success() {
                return true;
            }
        }
    }
    false
}

fn try_system_strip(src: &PathBuf, dst: &PathBuf) -> bool {
    // Copy then strip in-place.
    if fs::copy(src, dst).is_err() {
        return false;
    }
    Command::new("strip")
        .arg("--strip-debug")
        .arg(dst)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn write_boot_ramdisk(path: &PathBuf, boot_toml: &str) {
    fs::write(path, make_cpio_newc(&[("boot.toml", boot_toml.as_bytes())]))
        .expect("failed to create boot ramdisk");
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
