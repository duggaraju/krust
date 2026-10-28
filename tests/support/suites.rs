use super::qemu::*;
use super::shell::{Case, Check};
use regex::Regex;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;

pub fn assert_clean(guest: &Guest, output: &str) {
    assert!(!has_errors(output), "guest response: {output}");
    assert!(
        !has_kernel_errors(&guest.kernel_response()),
        "kernel response: {}",
        guest.kernel_response()
    );
}

pub fn kernel_address(trace: &str) -> Result<u64, String> {
    let capture = |pattern: &str| -> Result<u64, String> {
        let regex = Regex::new(pattern).unwrap();
        let hex = regex
            .captures(trace)
            .ok_or_else(|| format!("missing boot trace field: {pattern}"))?;
        u64::from_str_radix(&hex[1], 16).map_err(|error| error.to_string())
    };
    let bias = capture(r"virtual_address_offset: 0x([0-9a-fA-F]{1,12})\b")?;
    let segment =
        capture(r"Handling Segment:.*type_: Ok\(Load\).*virtual_addr: ([0-9a-fA-F]{1,12}),")?;
    let address = bias.checked_add(segment).ok_or("kernel address overflow")? & !4095;
    if address == 0 || address >= 1 << 47 {
        return Err("kernel address outside user range".into());
    }
    Ok(address)
}

pub fn smoke(guest: &mut Guest, probe: Option<&str>) {
    guest.boot();
    assert_eq!(guest.command("cd /"), "");
    if let Some(probe) = probe {
        let (command, markers) = match probe {
            "tmpfs" => ("/bin/krust-tmpfs-probe".to_string(), vec!["KRUST_TMPFS_OK"]),
            "cloexec" => (
                "cloexec-test".to_string(),
                vec![
                    "KRUST_CLOEXEC_BEFORE_OK",
                    "KRUST_CLOEXEC_EXEC_OK",
                    "KRUST_CLOEXEC_FORK_OK",
                ],
            ),
            "mmap" => (
                format!(
                    "/bin/krust-mmap-probe 0x{:x}",
                    kernel_address(&guest.kernel()).unwrap()
                ),
                vec!["KRUST_MMAP_OK", "KRUST_MMAP_KERNEL_OK"],
            ),
            _ => panic!("unknown guest probe: {probe}"),
        };
        let output = guest.command(&command);
        assert_clean(guest, &output);
        assert!(
            markers
                .iter()
                .all(|marker| output.lines().any(|line| line == *marker)),
            "{command}: {output}"
        );
    }
    let cases = [
        Case::exact("/bin/busybox pwd", "/"),
        Case::exact("/bin/busybox echo KRUST_ECHO_OK", "KRUST_ECHO_OK"),
        Case::new(
            "/bin/busybox ls /proc /dev",
            Check::Patterns(vec![
                r"(?m)^/proc:$",
                r"(?m)^/dev:$",
                r"(^|\s)meminfo(\s|$)",
                r"(^|\s)null(\s|$)",
            ]),
        ),
        Case::new(
            "/bin/busybox cat /proc/meminfo",
            Check::Patterns(vec![
                r"(?m)^MemTotal: +[1-9][0-9]* kB$",
                r"(?m)^MemFree: +[0-9]+ kB$",
                r"(?m)^Buffers:        0 kB$",
            ]),
        ),
        Case::new(
            "/bin/busybox cat /proc/cpuinfo",
            Check::Patterns(vec![r"(?m)^processor\t+: 0$", r"(?m)^vendor_id\t+: .+$"]),
        ),
        Case::new("/bin/busybox true", Check::Quiet),
        Case::exact("/bin/busybox echo KRUST_FORK_EXEC_OK", "KRUST_FORK_EXEC_OK"),
    ];
    for case in cases {
        guest.send(&case.command);
        let raw = guest.until("krust> ").expect("userspace command prompt");
        case.validate(&raw, &mut 0)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_clean(guest, &clean_response(&raw, &case.command));
    }
    guest.shutdown("exit");
    assert!(
        guest.kernel().contains("fork: parent_pid="),
        "missing fork trace"
    );
    assert!(!has_kernel_errors(&guest.kernel()), "{}", guest.kernel());
}

pub fn userspace() {
    let names = [
        "KRUST_SMOKE_SHELL_PROBE",
        "KRUST_SMOKE_MMAP_PROBE",
        "KRUST_SMOKE_CLOEXEC_PROBE",
        "KRUST_SMOKE_TMPFS_PROBE",
    ];
    let values = names.map(|name| {
        std::env::var_os(name)
            .map(|value| value.into_string().expect("ASCII probe flag"))
            .unwrap_or_else(|| "0".into())
    });
    let probes =
        probe_selection(values.each_ref().map(String::as_str)).expect("smoke probe selection");
    if probes == ["shell"] {
        super::shell::run();
        return;
    }
    let work = Workspace::new("userspace");
    let (disk, _unchanged) = copied_disk(&work);
    for firmware in FIRMWARE {
        for probe in &probes {
            smoke(
                &mut Guest::start(
                    &work,
                    &format!("{firmware}-{probe}"),
                    firmware,
                    Some(&disk),
                    &[],
                ),
                if *probe == "userspace" {
                    None
                } else {
                    Some(probe)
                },
            );
        }
    }
}

pub fn probe_selection(flags: [&str; 4]) -> Result<Vec<&'static str>, String> {
    if flags.iter().any(|flag| !["0", "1"].contains(flag)) {
        return Err("probe flags must be 0 or 1".into());
    }
    let selected = ["shell", "mmap", "cloexec", "tmpfs"]
        .into_iter()
        .zip(flags)
        .filter_map(|(name, flag)| (flag == "1").then_some(name))
        .collect::<Vec<_>>();
    if selected.contains(&"shell") && selected.len() > 1 {
        return Err("Run shell separately from guest probes".into());
    }
    if selected.is_empty() {
        Ok(vec!["userspace"])
    } else {
        Ok(selected)
    }
}

pub fn tmpfs() {
    let work = Workspace::new("tmpfs");
    prepare_root(&work);
    for dir in ["tmp", "run", "tiny", "large", "parent/child"] {
        fs::create_dir_all(work.root().join(dir)).unwrap();
    }
    fs::copy(
        env!("CARGO_BIN_FILE_GUEST_tmpfs"),
        work.root().join("bin/krust-tmpfs-probe"),
    )
    .unwrap();
    fs::write(
        work.root().join("etc/fstab"),
        "\
/dev/sda / ext4 defaults 0 0
proc /proc proc defaults 0 0
dev /dev dev defaults 0 0
tmpfs /tmp tmpfs size=8k,nr_inodes=8,mode=1777 0 0
none /run tmpfs size=4k,nr_inodes=2 0 0
none /tiny tmpfs size=4k,nr_inodes=2 0 0
none /large tmpfs size=4M,nr_inodes=2 0 0
none /parent/child tmpfs size=4k,nr_inodes=2 0 0
none /parent tmpfs size=4k,nr_inodes=4 0 0
",
    )
    .unwrap();
    let disk = work.mkdisk("ext4");
    let _unchanged = UnchangedDisk::new(&disk);
    for firmware in FIRMWARE {
        smoke(
            &mut Guest::start(&work, firmware, firmware, Some(&disk), &[]),
            Some("tmpfs"),
        );
    }
}

pub fn copied_disk(work: &Workspace) -> (std::path::PathBuf, UnchangedDisk) {
    let source = fixture("KRUST_TEST_DISK", "target/krust-disk-ext4.vhd");
    let unchanged = UnchangedDisk::new(&source);
    let disk = work.scratch.join("disk.vhd");
    fs::copy(source, &disk).unwrap();
    checked(
        Command::new("debugfs")
            .args(["-w", "-R", "rm /sbin/init"])
            .arg(&disk),
    );
    let init = checked(
        Command::new("debugfs")
            .args(["-R", "stat /sbin/init"])
            .arg(&disk),
    );
    assert!(
        init.stdout.is_empty(),
        "kernel-shell fixture still contains /sbin/init"
    );
    (disk, unchanged)
}

pub fn cloexec() {
    let work = Workspace::new("cloexec");
    let (disk, _source_unchanged) = copied_disk(&work);
    fs::write(work.scratch.join("data"), b"abcdef\n").unwrap();
    for (source, destination) in [
        (
            Path::new(env!("CARGO_BIN_FILE_GUEST_cloexec_before")),
            "/bin/krust-cloexec-before",
        ),
        (
            Path::new(env!("CARGO_BIN_FILE_GUEST_cloexec_after")),
            "/bin/krust-cloexec-after",
        ),
        (&work.scratch.join("data"), "/bin/krust-cloexec-data"),
    ] {
        inject(&disk, source, destination);
    }
    let _unchanged = UnchangedDisk::new(&disk);
    for firmware in FIRMWARE {
        smoke(
            &mut Guest::start(&work, firmware, firmware, Some(&disk), &[]),
            Some("cloexec"),
        );
    }
}

pub const PROTECTION_MODES: [(&str, &str); 9] = [
    (
        "ro-write",
        "PROTECTION_VIOLATION | CAUSED_BY_WRITE | USER_MODE",
    ),
    (
        "exec-write",
        "PROTECTION_VIOLATION | CAUSED_BY_WRITE | USER_MODE",
    ),
    (
        "rx-write",
        "PROTECTION_VIOLATION | CAUSED_BY_WRITE | USER_MODE",
    ),
    (
        "nx-read",
        "PROTECTION_VIOLATION | USER_MODE | INSTRUCTION_FETCH",
    ),
    (
        "nx-write",
        "PROTECTION_VIOLATION | USER_MODE | INSTRUCTION_FETCH",
    ),
    (
        "nx-rw",
        "PROTECTION_VIOLATION | USER_MODE | INSTRUCTION_FETCH",
    ),
    ("none-read", "USER_MODE"),
    ("none-write", "CAUSED_BY_WRITE | USER_MODE"),
    ("none-exec", "USER_MODE | INSTRUCTION_FETCH"),
];

pub fn protection_response(
    mode: &str,
    flags: &str,
    output: &str,
    kernel: &str,
) -> Result<(), String> {
    let status = "/bin/krust-mmap-protection: exited with status 139";
    if !output
        .lines()
        .any(|line| line == format!("KRUST_PROTECTION_ARMED {mode}"))
        || output.lines().filter(|line| *line == status).count() != 1
    {
        return Err(format!("missing armed marker/fault status: {output}"));
    }
    let filtered = output
        .lines()
        .filter(|line| *line != status)
        .collect::<Vec<_>>()
        .join("\n");
    if has_errors(&filtered) || filtered.contains("KRUST_PROTECTION_FAIL") {
        return Err(format!("unexpected protection response: {output}"));
    }
    let regex = Regex::new(r"page fault in user task pid=[0-9]+ - terminating task").unwrap();
    if kernel.matches("EXCEPTION: PAGE FAULT").count() != 1
        || regex.find_iter(kernel).count() != 1
        || !kernel
            .lines()
            .any(|line| line == format!("ERROR: Error Code: PageFaultErrorCode({flags})"))
    {
        return Err(format!("incorrect user fault trace: {kernel}"));
    }
    let filtered = kernel
        .lines()
        .filter(|line| *line != "ERROR: EXCEPTION: PAGE FAULT")
        .collect::<Vec<_>>()
        .join("\n");
    if has_kernel_errors(&filtered) || filtered.contains("KRUST_PROTECTION_FAIL") {
        return Err(format!("unexpected fault diagnostic: {filtered}"));
    }
    Ok(())
}

pub fn mmap() {
    let work = Workspace::new("mmap");
    let (disk, _source_unchanged) = copied_disk(&work);
    inject(
        &disk,
        Path::new(env!("CARGO_BIN_FILE_GUEST_mmap")),
        "/bin/krust-mmap-probe",
    );
    inject(
        &disk,
        Path::new(env!("CARGO_BIN_FILE_GUEST_mmap_protection")),
        "/bin/krust-mmap-protection",
    );
    let _unchanged = UnchangedDisk::new(&disk);
    for firmware in FIRMWARE {
        let mut guest = Guest::start(
            &work,
            &format!("{firmware}-protection"),
            firmware,
            Some(&disk),
            &[],
        );
        guest.boot();
        assert_eq!(guest.command("cd /"), "");
        assert_eq!(
            guest.command("/bin/krust-mmap-protection ok"),
            "KRUST_PROTECTION_OK"
        );
        assert_clean(&guest, "KRUST_PROTECTION_OK");
        for (mode, flags) in PROTECTION_MODES {
            let output = guest.command(&format!("/bin/krust-mmap-protection {mode}"));
            protection_response(mode, flags, &output, &guest.kernel_response())
                .unwrap_or_else(|error| panic!("{firmware} {mode}: {error}"));
            let output = guest.command("/bin/busybox echo KRUST_PARENT_OK");
            assert_eq!(output, "KRUST_PARENT_OK");
            assert_clean(&guest, &output);
        }
        assert_eq!(guest.kernel().matches("EXCEPTION:").count(), 9);
        guest.shutdown("exit");
        assert!(
            !has_kernel_errors(&guest.kernel_response()),
            "{}",
            guest.kernel_response()
        );
        drop(guest);
        smoke(
            &mut Guest::start(&work, firmware, firmware, Some(&disk), &[]),
            Some("mmap"),
        );
    }
}

pub fn install(work: &Workspace, format: &str) {
    checked(
        Command::new("bash")
            .arg("scripts/install-busybox-applets.sh")
            .arg(fixture("KRUST_TEST_BUSYBOX", "target/rootfs/bin/busybox"))
            .arg(work.root())
            .arg(format),
    );
}

pub fn trace_syscalls(disk: &Path) {
    let path = disk.with_extension("vhd.boot.toml");
    let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(file, "syscall_trace = true").unwrap();
}

pub fn shell_process() {
    let work = Workspace::new("process");
    install(&work, "ext4");
    assert!(work.root().join("bin/sh").is_symlink());
    install(&work, "fat");
    let busybox = fixture("KRUST_TEST_BUSYBOX", "target/rootfs/bin/busybox");
    for shell in ["sh", "ash"] {
        let path = work.root().join("bin").join(shell);
        assert!(!path.is_symlink());
        assert_eq!(fs::read(&path).unwrap(), fs::read(&busybox).unwrap());
        assert_eq!(
            checked(Command::new(path).args(["-c", "echo HOST_SHELL_OK"])).stdout,
            b"HOST_SHELL_OK\n"
        );
    }
    assert!(!work.root().join("bin/bash").exists());
    assert_eq!(
        fs::read_to_string(work.root().join("bin/ls"))
            .unwrap()
            .lines()
            .next(),
        Some("#!/bin/busybox sh")
    );
    for (name, interpreter) in [
        ("selfloop", "selfloop"),
        ("cyclea", "cycleb"),
        ("cycleb", "cyclea"),
        ("chain1", "chain2"),
        ("chain2", "chain3"),
        ("chain3", "chain4"),
        ("chain4", "chain5"),
        ("chain5", "busybox echo"),
        ("direct", "busybox basename"),
        ("nested1", "nested2"),
        ("nested2", "nested3"),
        ("nested3", "nested4"),
        ("nested4", "busybox echo"),
    ] {
        fs::write(
            work.root().join("bin").join(name),
            format!("#!/bin/{interpreter}\n"),
        )
        .unwrap();
    }
    fs::copy(
        env!("CARGO_BIN_FILE_GUEST_process"),
        work.root().join("bin/proctest"),
    )
    .unwrap();
    let disk = work.mkdisk("fat");
    trace_syscalls(&disk);
    let _unchanged = UnchangedDisk::new(&disk);
    for firmware in FIRMWARE {
        let mut guest = Guest::start(&work, firmware, firmware, Some(&disk), &[]);
        guest.boot();
        let baseline = guest.command("ps");
        for (command, expected) in [
            ("/bin/sh -c pwd", "/"),
            ("/bin/ash -c pwd", "/"),
            ("/bin/proctest", "KRUST_CPU_ACCOUNTING_OK\nKRUST_PROCESS_OK"),
            ("/bin/direct", "direct"),
            (
                "/bin/nested1",
                "/bin/nested4 /bin/nested3 /bin/nested2 /bin/nested1",
            ),
        ] {
            assert_eq!(guest.command(command), expected);
        }
        let output = guest.command("/bin/busybox sh -c ls;ls;echo${IFS}KRUST_ASH_SUBPROCESS_OK");
        subprocess_listing(&output, "KRUST_ASH_SUBPROCESS_OK");
        for (command, marker) in [
            ("/bin/selfloop", "KRUST_AFTER_SELF_LOOP"),
            ("/bin/cyclea", "KRUST_AFTER_CYCLE"),
            ("/bin/chain1", "KRUST_AFTER_LIMIT"),
        ] {
            assert_eq!(
                guest.command(command),
                format!("{command}: exited with status 127")
            );
            assert_eq!(guest.command(&format!("echo {marker}")), marker);
        }
        assert_eq!(guest.command("/bin/sh -c pwd"), "/");
        guest.send("/bin/busybox sh");
        let mut interactive = guest.until("# ").expect("interactive shell prompt");
        for command in ["ls", "ls", "/bin/busybox ps", "echo KRUST_INTERACTIVE_OK"] {
            guest.send(command);
            interactive.push_str(&guest.until("# ").expect("interactive command prompt"));
        }
        guest.send("exit");
        interactive.push_str(&guest.until("krust> ").expect("return to kernel shell"));
        subprocess_listing(
            &clean_response(&interactive, "/bin/busybox sh"),
            "KRUST_INTERACTIVE_OK",
        );
        assert!(
            interactive.contains("PID") && interactive.contains("busybox"),
            "{interactive}"
        );
        assert_eq!(
            guest.command("ps"),
            baseline,
            "exited children must be reaped"
        );
        guest.shutdown("exit");
        let trace = guest.kernel();
        assert_eq!(
            trace.matches("interpreter redirect limit exceeded").count(),
            3
        );
        assert_eq!(trace.matches("errno -40").count(), 3);
        assert_eq!(trace.matches("EXCEPTION:").count(), 1);
        assert_eq!(trace.matches("EXCEPTION: PAGE FAULT").count(), 1);
        assert_eq!(
            Regex::new(r"Error Code:.*USER_MODE")
                .unwrap()
                .find_iter(&trace)
                .count(),
            1
        );
        assert_eq!(trace.matches("page fault in user task").count(), 1);
        assert!(!trace.contains("Kernel panic") && !trace.contains("panicked"));
    }
}

fn subprocess_listing(output: &str, marker: &str) {
    assert!(
        output
            .lines()
            .any(|line| line.trim_start_matches("# ") == marker),
        "{output}"
    );
    assert_eq!(
        output
            .lines()
            .filter(|line| line.split_whitespace().any(|word| word == "bin"))
            .count(),
        2,
        "{output}"
    );
    assert!(
        !matches(r"not found|Usage:|errno|status", output),
        "{output}"
    );
}

pub fn init_process() {
    let work = Workspace::new("init");
    prepare_root(&work);
    fs::create_dir(work.root().join("sbin")).unwrap();
    for name in ["initest", "altinit", "exitinit", "faultini"] {
        fs::copy(
            env!("CARGO_BIN_FILE_GUEST_init_probe"),
            work.root().join("bin").join(name),
        )
        .unwrap();
    }
    fs::write(work.root().join("bin/badinit"), "not an executable\n").unwrap();
    for format in ["fat", "ext4"] {
        let init = work.root().join("sbin/init");
        let default_argv0 = if format == "ext4" {
            std::os::unix::fs::symlink("/bin/initest", &init).unwrap();
            "/sbin/init"
        } else {
            fs::write(&init, "#!/bin/initest\n").unwrap();
            "/bin/initest"
        };
        let disk = work.mkdisk(format);
        {
            let _unchanged = UnchangedDisk::new(&disk);
            for firmware in FIRMWARE {
                for (selection, expected) in [
                    (None, default_argv0),
                    (Some("init=/bin/altinit"), "/bin/altinit"),
                ] {
                    let extra = selection.map(std::ffi::OsStr::new);
                    let extra = extra.iter().copied().collect::<Vec<_>>();
                    let mut guest = Guest::start(
                        &work,
                        &format!(
                            "{format}-{firmware}-{}",
                            if selection.is_some() {
                                "override"
                            } else {
                                "default"
                            }
                        ),
                        firmware,
                        Some(&disk),
                        &extra,
                    );
                    guest.finish(0);
                    assert!(
                        guest
                            .console
                            .lines()
                            .any(|line| line == format!("KRUST_INIT_ARGV0={expected}"))
                            && guest.console.contains("KRUST_INIT_PID1_OK"),
                        "{}",
                        guest.console
                    );
                    assert!(!guest.console.contains("krust kernel shell"));
                    assert!(!has_kernel_errors(&guest.kernel()), "{}", guest.kernel());
                    assert!(guest.kernel().contains("execve: entering usermode pid=1"));
                }
                if format == "ext4" {
                    for (path, expected) in [
                        ("/bin/missing", "errno -2"),
                        ("/bin/badinit", "errno -8"),
                        ("/bin/exitinit", "PID 1 terminated with status 42"),
                        ("/bin/faultini", "PID 1 terminated with status 139"),
                    ] {
                        let argument = format!("init={path}");
                        let mut guest = Guest::start(
                            &work,
                            &format!("{format}-{firmware}-{}", path.rsplit('/').next().unwrap()),
                            firmware,
                            Some(&disk),
                            &[argument.as_ref()],
                        );
                        guest.finish(1);
                        assert!(guest.kernel().contains(expected), "{}", guest.kernel());
                        assert!(!guest.console.contains("krust kernel shell"));
                    }
                }
            }
        }
        fs::remove_file(init).unwrap();
        let disk = work.mkdisk(format);
        let _unchanged = UnchangedDisk::new(&disk);
        for firmware in FIRMWARE {
            let mut guest = Guest::start(
                &work,
                &format!("{format}-{firmware}-fallback"),
                firmware,
                Some(&disk),
                &[],
            );
            guest.boot();
            let tasks = guest.command("ps");
            assert!(matches(r"(?m)^1 +0 +Running +shell$", &tasks), "{tasks}");
            assert!(!matches(r"(?m)^[0-9]+ +[0-9]+ +[^ ]+ +root$", &tasks));
            assert_eq!(guest.command("/bin/busybox sh -c pwd"), "/");
            guest.shutdown("exit");
            assert!(!has_kernel_errors(&guest.kernel()), "{}", guest.kernel());
        }
    }
}

pub fn init_shell() {
    let work = Workspace::new("init-shell");
    prepare_root(&work);
    fs::create_dir(work.root().join("sbin")).unwrap();
    fs::write(
        work.root().join("sbin/init"),
        "#!/bin/busybox sh\nexec /bin/busybox sh \"$@\"\n",
    )
    .unwrap();
    for format in ["fat", "ext4"] {
        let disk = work.mkdisk(format);
        let _unchanged = UnchangedDisk::new(&disk);
        for firmware in FIRMWARE {
            let mut guest = Guest::start(
                &work,
                &format!("{format}-{firmware}"),
                firmware,
                Some(&disk),
                &[],
            );
            guest.until("# ").expect("PID 1 userspace shell prompt");
            guest.send("echo KRUST_INIT_SHELL_PID=$$");
            let output = guest.until("# ").expect("PID 1 echo response");
            assert_eq!(
                output.matches("echo KRUST_INIT_SHELL_PID=$$").count(),
                1,
                "{output}"
            );
            assert!(
                output.lines().any(|line| line == "KRUST_INIT_SHELL_PID=1"),
                "{output}"
            );
            guest.send("/bin/busybox pwd");
            let output = guest.until("# ").expect("PID 1 external command response");
            assert!(output.lines().any(|line| line == "/"), "{output}");
            guest.send("exit");
            guest.finish(1);
            assert!(guest.kernel().contains("PID 1 terminated with status 0"));
            assert!(!has_kernel_errors(&guest.kernel()), "{}", guest.kernel());
        }
    }
}

pub fn terminal_console() {
    let work = Workspace::new("terminal");
    prepare_root(&work);
    fs::copy(
        env!("CARGO_BIN_FILE_GUEST_init_probe"),
        work.root().join("bin/ttytest"),
    )
    .unwrap();
    for format in ["fat", "ext4"] {
        let disk = work.mkdisk(format);
        let _unchanged = UnchangedDisk::new(&disk);
        for firmware in FIRMWARE {
            let mut guest = Guest::start(
                &work,
                &format!("{format}-{firmware}"),
                firmware,
                Some(&disk),
                &[std::ffi::OsStr::new("init=/bin/ttytest")],
            );
            for _ in 0..2 {
                guest
                    .until("KRUST_TTY_RAW_READY")
                    .expect("raw terminal ready");
                std::thread::sleep(std::time::Duration::from_millis(1200));
                guest.send_bytes(b"A");
                let output = guest
                    .until("KRUST_TTY_COOKED_READY")
                    .expect("raw read finished");
                assert!(
                    !output
                        .strip_suffix("KRUST_TTY_COOKED_READY")
                        .unwrap()
                        .contains('A'),
                    "{output}"
                );
                guest.send_bytes(b"abc");
                std::thread::sleep(std::time::Duration::from_millis(200));
                guest.send_bytes(b"\n");
                let output = guest
                    .until("KRUST_TTY_QUIET_READY")
                    .expect("canonical read finished");
                assert_eq!(output.matches("abc").count(), 1, "{output}");
                guest.send_bytes(b"secret\n");
                let output = guest
                    .until("KRUST_TTY_DEVICE_OK")
                    .expect("quiet read finished");
                assert!(!output.contains("secret"), "{output}");
            }
            guest.finish(0);
            assert!(!has_kernel_errors(&guest.kernel()), "{}", guest.kernel());
        }
    }
}

pub fn boot_mounts() {
    let work = Workspace::new("mounts");
    prepare_root(&work);
    for format in ["fat", "ext4"] {
        let fstab = format!(
            "/dev/sda / {format} defaults 0 0\n\
none /scratch/child ramfs defaults 0 0\nnone /scratch ramfs defaults 0 0\n\
/dev/not-present /offline fat noauto 0 0\nproc /proc proc defaults 0 0\ndev /dev dev defaults 0 0\n"
        );
        let override_path = work.scratch.join("ext4.fstab");
        if format == "fat" {
            fs::write(work.root().join("etc/fstab"), &fstab).unwrap();
        } else {
            fs::write(&override_path, &fstab).unwrap();
        }
        let before = fs::read(work.root().join("etc/fstab")).unwrap();
        let disk = work.scratch.join(format!("{format}.vhd"));
        let extra = if format == "ext4" {
            vec![std::ffi::OsStr::new("--fstab"), override_path.as_os_str()]
        } else {
            vec![]
        };
        work.mkdisk_at(format, &disk, &extra);
        assert_eq!(fs::read(work.root().join("etc/fstab")).unwrap(), before);
        let config = fs::read_to_string(disk.with_extension("vhd.boot.toml")).unwrap();
        assert!(
            config
                .lines()
                .any(|line| line == format!("root_fs = \"{format}\""))
        );
        let _unchanged = UnchangedDisk::new(&disk);
        for firmware in FIRMWARE {
            let mut guest = Guest::start(
                &work,
                &format!("{format}-{firmware}"),
                firmware,
                Some(&disk),
                &[],
            );
            smoke(&mut guest, None);
            let trace = guest.kernel();
            for expected in [
                format!("mount: mounted filesystem '{format}' on '/'"),
                "fstab: root entry matches early boot settings".into(),
                "fstab: skipping noauto mount '/offline'".into(),
            ] {
                assert!(trace.contains(&expected), "{expected}");
            }
            let parent = trace
                .find("mount: mounted filesystem 'ramfs' on '/scratch'")
                .unwrap();
            let child = trace
                .find("mount: mounted filesystem 'ramfs' on '/scratch/child'")
                .unwrap();
            assert!(parent < child);
        }
    }
    let config = work.scratch.join("missing.toml");
    fs::write(
        &config,
        "root_fs = \"ext4\"\nroot_device = \"/dev/not-present\"\n",
    )
    .unwrap();
    for firmware in FIRMWARE {
        let guest = Guest::start(
            &work,
            &format!("{firmware}-missing-root"),
            firmware,
            None,
            &["--boot-config".as_ref(), config.as_os_str()],
        );
        boot_failure(guest, "root ext4 on /dev/not-present: NotFound");
    }
    let disk = work.scratch.join("invalid.vhd");
    fs::copy(work.scratch.join("ext4.vhd"), &disk).unwrap();
    for (name, contents, expected) in [
        (
            "option",
            "proc /proc proc ro 0 0\n",
            "unsupported mount option",
        ),
        (
            "duplicate",
            "proc /proc proc defaults 0 0\nproc /proc proc defaults 0 0\n",
            "duplicate mount target",
        ),
        (
            "mismatch",
            "/dev/other / ext4 defaults 0 0\n",
            "fstab root entry disagrees",
        ),
        (
            "missing-mount",
            "/dev/not-present /offline fat defaults 0 0\n",
            "fstab: /dev/not-present on /offline (fat): NotFound",
        ),
        (
            "malformed",
            "proc /proc proc defaults\n",
            "expected six fields",
        ),
    ] {
        let source = work.scratch.join("invalid.fstab");
        fs::write(&source, contents).unwrap();
        inject(&disk, &source, "/etc/fstab");
        let readback = checked(
            Command::new("debugfs")
                .args(["-R", "cat /etc/fstab"])
                .arg(&disk),
        );
        assert_eq!(readback.stdout, contents.as_bytes());
        let _unchanged = UnchangedDisk::new(&disk);
        for firmware in FIRMWARE {
            boot_failure(
                Guest::start(
                    &work,
                    &format!("{firmware}-{name}"),
                    firmware,
                    Some(&disk),
                    &[],
                ),
                expected,
            );
        }
    }
    fs::write(&config, "root_fs = \"unknown\"\n").unwrap();
    let mut guest = Guest::start(
        &work,
        "unknown",
        "bios",
        None,
        &["--boot-config".as_ref(), config.as_os_str()],
    );
    guest.finish(1);
    assert!(
        guest.console.contains("unknown setting or invalid value"),
        "{}",
        guest.console
    );
    assert!(!guest.console.contains("headless kernel trace log:"));
}

fn boot_failure(mut guest: Guest, expected: &str) {
    guest.finish(1);
    assert!(guest.console.contains("guest 0x11"), "{}", guest.console);
    let trace = guest.kernel();
    assert!(
        trace.contains("boot mounts failed:") && trace.contains(expected),
        "{expected}: {trace}"
    );
    assert!(!trace.contains("fstab: configured mounts ready"));
}
