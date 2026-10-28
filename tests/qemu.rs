mod support;

const GUEST_BINARIES: [(&str, &str); 6] = [
    ("KRUST_GUEST_TMPFS", env!("CARGO_BIN_FILE_GUEST_tmpfs")),
    (
        "KRUST_GUEST_CLOEXEC_BEFORE",
        env!("CARGO_BIN_FILE_GUEST_cloexec_before"),
    ),
    (
        "KRUST_GUEST_CLOEXEC_AFTER",
        env!("CARGO_BIN_FILE_GUEST_cloexec_after"),
    ),
    ("KRUST_GUEST_MMAP", env!("CARGO_BIN_FILE_GUEST_mmap")),
    (
        "KRUST_GUEST_MMAP_PROTECTION",
        env!("CARGO_BIN_FILE_GUEST_mmap_protection"),
    ),
    ("KRUST_GUEST_PROCESS", env!("CARGO_BIN_FILE_GUEST_process")),
];

#[test]
fn guest_artifacts_are_static_user_executables() {
    for (name, path) in GUEST_BINARIES {
        let elf = std::fs::read(path).unwrap_or_else(|error| panic!("{name}: {error}"));
        let u16_at = |offset| u16::from_le_bytes(elf[offset..offset + 2].try_into().unwrap());
        let u32_at = |offset| u32::from_le_bytes(elf[offset..offset + 4].try_into().unwrap());
        let u64_at = |offset| u64::from_le_bytes(elf[offset..offset + 8].try_into().unwrap());
        assert_eq!(&elf[..7], b"\x7fELF\x02\x01\x01", "{name}");
        assert_eq!(u16_at(16), 2, "{name}: executable, not PIE");
        assert_eq!(u16_at(18), 62, "{name}: x86_64");
        assert_eq!(u16_at(54), 56, "{name}: ELF64 program headers");
        let entry = u64_at(24);
        let headers = u64_at(32) as usize;
        let mut executable_entry = false;
        for index in 0..u16_at(56) as usize {
            let header = headers + index * 56;
            let kind = u32_at(header);
            assert!(
                !matches!(kind, 2 | 3),
                "{name}: no dynamic loader or dynamic segment"
            );
            if kind != 1 {
                continue;
            }
            let flags = u32_at(header + 4);
            let offset = u64_at(header + 8);
            let address = u64_at(header + 16);
            let file_size = u64_at(header + 32);
            let memory_size = u64_at(header + 40);
            assert!(
                address >= 0x400000 && address + memory_size <= 1 << 47,
                "{name}"
            );
            assert!(
                file_size <= memory_size && offset + file_size <= elf.len() as u64,
                "{name}"
            );
            assert_ne!(flags & 3, 3, "{name}: writable code");
            executable_entry |= flags & 1 != 0 && (address..address + memory_size).contains(&entry);
        }
        assert!(
            executable_entry,
            "{name}: entry point outside executable segment"
        );
    }
}

fn utility_matrix(format: &str, firmware: &str) {
    support::utilities::run(format, firmware, &[]);
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
    ($name:ident, $run:path) => {
        #[test]
        #[ignore = "requires QEMU, existing fixtures, and the suite's host tools"]
        fn $name() {
            $run();
        }
    };
}

guest_suite!(shell_process, support::suites::shell_process);
guest_suite!(boot_mounts, support::suites::boot_mounts);
guest_suite!(tmpfs, support::suites::tmpfs);
guest_suite!(shell_commands, support::shell::run);
guest_suite!(userspace_smoke, support::suites::userspace);
guest_suite!(cloexec, support::suites::cloexec);
guest_suite!(mmap, support::suites::mmap);
