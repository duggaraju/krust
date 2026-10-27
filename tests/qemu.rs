mod support;

fn utility_matrix(format: &str, firmware: &str) {
    let output = format!("target/cargo-tests/qemu/{format}/{firmware}");
    support::run("scripts/test-busybox-utilities.sh", |command| {
        command
            .arg(support::fixture(
                "KRUST_TEST_BUSYBOX",
                "target/rootfs/bin/busybox",
            ))
            .arg(env!("CARGO_BIN_EXE_krust"))
            .args([format, "--firmware", firmware, "--output-dir", &output]);
    });
}

macro_rules! matrix {
    ($name:ident, $format:literal, $firmware:literal) => {
        #[test]
        #[ignore = "full installed-utility matrix; requires QEMU/BusyBox; known gaps fail"]
        fn $name() {
            utility_matrix($format, $firmware);
        }
    };
}

matrix!(busybox_fat_bios, "fat", "bios");
matrix!(busybox_fat_uefi, "fat", "uefi");
matrix!(busybox_ext4_bios, "ext4", "bios");
matrix!(busybox_ext4_uefi, "ext4", "uefi");

macro_rules! guest_suite {
    ($name:ident, $script:literal, $variable:literal, $default:literal) => {
        #[test]
        #[ignore = "requires QEMU, existing fixtures, and the suite's host tools"]
        fn $name() {
            support::run($script, |command| {
                command
                    .arg(support::fixture($variable, $default))
                    .arg(env!("CARGO_BIN_EXE_krust"));
            });
        }
    };
}

guest_suite!(
    shell_process,
    "scripts/test-fat-shell.sh",
    "KRUST_TEST_BUSYBOX",
    "target/rootfs/bin/busybox"
);
guest_suite!(
    boot_mounts,
    "scripts/test-boot-mounts.sh",
    "KRUST_TEST_BUSYBOX",
    "target/rootfs/bin/busybox"
);
guest_suite!(
    shell_commands,
    "scripts/smoke-shell.sh",
    "KRUST_TEST_DISK",
    "target/krust-disk-ext4.vhd"
);
guest_suite!(
    userspace_smoke,
    "scripts/smoke-userspace.sh",
    "KRUST_TEST_DISK",
    "target/krust-disk-ext4.vhd"
);
guest_suite!(
    cloexec,
    "scripts/test-cloexec-guest.sh",
    "KRUST_TEST_DISK",
    "target/krust-disk-ext4.vhd"
);
guest_suite!(
    mmap,
    "scripts/test-mmap-guest.sh",
    "KRUST_TEST_DISK",
    "target/krust-disk-ext4.vhd"
);
