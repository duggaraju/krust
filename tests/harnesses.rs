mod support;

use std::fs;
use std::process::Command;
use std::time::{Duration, Instant};
use support::qemu::*;
use support::shell::{Case, Check};
use support::suites::{PROTECTION_MODES, kernel_address, protection_response};
use support::utilities::{Validation, case};

fn mock(work: &Workspace, mode: &str, seconds: f64) -> Guest {
    let mut command = Command::new(env!("CARGO_BIN_FILE_MOCK_mock_runner"));
    command.arg(mode);
    Guest::spawn(work, mode, command, Duration::from_secs_f64(seconds))
}

#[test]
fn fragmented_prompts_and_input_echo() {
    let work = Workspace::new("mock-fragmented");
    let mut guest = mock(&work, "fragmented", 5.0);
    guest.boot();
    assert_eq!(guest.command("/bin/busybox pwd"), "/");
    assert_eq!(
        guest.command("/bin/busybox echo KRUST_ECHO_OK"),
        "KRUST_ECHO_OK"
    );
    guest.shutdown("exit");
    assert_eq!(
        clean_response("\x1b[32mecho x\r\nx\r\n\x1b[0mkrust> ", "echo x"),
        "x"
    );
    assert_eq!(
        clean_response("echo x\necho x\nkrust> ", "echo x"),
        "echo x"
    );
}

#[test]
fn echoed_commands_cannot_satisfy_assertions() {
    let work = Workspace::new("mock-echo");
    let mut guest = mock(&work, "echo-only", 5.0);
    guest.boot();
    let output = guest.command("/bin/busybox echo KRUST_ECHO_OK");
    assert!(
        Case::exact("/bin/busybox echo KRUST_ECHO_OK", "KRUST_ECHO_OK")
            .validate(&output, &mut 0)
            .is_err()
    );
    assert!(
        Case::exact("/bin/busybox pwd", "/")
            .validate("/bin/busybox pwd\nkrust> ", &mut 0)
            .is_err()
    );
    guest.shutdown("exit");
}

#[test]
fn missing_trace_and_kernel_errors_reject_boot() {
    for mode in ["no-trace", "kernel-error"] {
        let work = Workspace::new(mode);
        assert!(mock(&work, mode, 5.0).try_boot().is_err(), "{mode}");
    }
    assert!(!has_kernel_errors(
        "ERROR: syscall failed: nr=9 (mmap) errno=-22"
    ));
    for line in [
        "ERROR: syscall failed: nr=9 (mmap) errno=-22 extra",
        "ERROR: errno -22",
        "EXCEPTION: PAGE FAULT",
        "panicked",
        "Kernel panic",
    ] {
        assert!(has_kernel_errors(line), "{line}");
    }
}

#[test]
fn prompt_timeout_and_eof_without_exit_are_bounded() {
    for mode in ["no-prompt", "closed-pipes-live"] {
        let work = Workspace::new(mode);
        let start = Instant::now();
        let mut guest = mock(&work, mode, 0.3);
        if mode == "no-prompt" {
            assert!(guest.try_boot().is_err());
        } else {
            guest.boot();
            guest.send("exit");
            assert!(guest.try_finish(0).unwrap_err().contains("timeout"));
        }
        drop(guest);
        assert!(start.elapsed() < Duration::from_secs(3), "{mode}");
    }
}

#[test]
fn nonzero_runner_exit_is_not_success() {
    let work = Workspace::new("mock-status");
    let mut guest = mock(&work, "exit-failure", 5.0);
    guest.boot();
    guest.send("exit");
    assert!(guest.try_finish(0).unwrap_err().contains("expected 0"));
}

#[test]
fn per_command_timeout_allows_a_fresh_guest() {
    let work = Workspace::new("mock-command-timeout");
    let mut guest = mock(&work, "command-timeout", 5.0);
    guest.boot();
    let start = Instant::now();
    assert!(
        guest
            .command_for("/bin/busybox pwd", Duration::from_millis(100))
            .is_err()
    );
    assert!(start.elapsed() < Duration::from_secs(2));
    drop(guest);
    let mut guest = mock(&work, "healthy", 5.0);
    guest.boot();
    assert_eq!(guest.command("/bin/busybox pwd"), "/");
    guest.shutdown("exit");
}

#[test]
fn shutdown_diagnostics_are_not_success() {
    let work = Workspace::new("mock-shutdown");
    let mut guest = mock(&work, "shutdown-error", 5.0);
    guest.boot();
    assert!(guest.try_shutdown("exit").unwrap_err().contains("errno -5"));
}

#[test]
fn dropping_guest_terminates_owned_children_only() {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    let work = Workspace::new("mock-cleanup");
    let sentinel = mock(&work, "child", 5.0);
    let mut guest = mock(&work, "linger-child", 5.0);
    guest.boot();
    let child = fs::read_to_string(work.scratch.join("child.pid"))
        .unwrap()
        .parse::<i32>()
        .unwrap();
    drop(guest);
    let child_state = fs::read_to_string(format!("/proc/{child}/stat"));
    assert!(
        child_state.is_err() || child_state.unwrap().split_whitespace().nth(2) == Some("Z"),
        "owned child still running"
    );
    let alive = kill(Pid::from_raw(sentinel.id() as i32), None).is_ok();
    assert!(alive, "unrelated sentinel was killed");
    assert!(work.output.join("linger-child-kernel.log").is_file());
}

#[test]
fn timeout_and_probe_selection_reject_invalid_values() {
    for value in ["", "0", "121", "-1", "1.5", "03", "1s", "1;exit"] {
        assert!(parse_timeout(value).is_err());
    }
    for value in ["1", "120"] {
        assert!(parse_timeout(value).is_ok());
    }
    use support::suites::probe_selection;
    assert_eq!(probe_selection(["0"; 4]).unwrap(), ["userspace"]);
    assert_eq!(probe_selection(["1", "0", "0", "0"]).unwrap(), ["shell"]);
    assert_eq!(
        probe_selection(["0", "1", "1", "1"]).unwrap(),
        ["mmap", "cloexec", "tmpfs"]
    );
    assert!(probe_selection(["1", "1", "0", "0"]).is_err());
    for bad in ["", "yes", "2"] {
        assert!(probe_selection([bad, "0", "0", "0"]).is_err());
    }
}

#[test]
fn source_disk_guard_checks_failure_paths() {
    let work = Workspace::new("mock-disk");
    let path = work.scratch.join("disk");
    fs::write(&path, "original").unwrap();
    let before = digest(&path);
    let result = std::panic::catch_unwind(|| {
        let _disk = UnchangedDisk::new(&path);
        fs::write(&path, "changed").unwrap();
    });
    assert!(result.is_err());
    assert_ne!(before, digest(&path));
}

#[test]
fn shell_case_inventory_and_recovery() {
    for (ramfs, count, cwd) in [(false, 33, "/"), (true, 77, "/krust-shell-test/nested")] {
        let cases = support::shell::cases(ramfs);
        assert_eq!(cases.len(), count);
        for group in cases
            .windows(3)
            .filter(|group| group[1].command == "echo KRUST_SHELL_RECOVERY_OK")
        {
            assert!(matches!(group[0].check, Check::Exact(_)));
            assert_eq!(group[2].command, "pwd");
            assert!(
                group[1]
                    .validate("KRUST_SHELL_RECOVERY_OK\nkrust> ", &mut 0)
                    .is_ok()
            );
            assert!(
                group[1]
                    .validate("KRUST_SHELL_RECOVERY_BAD\nkrust> ", &mut 0)
                    .is_err()
            );
            assert!(
                group[2]
                    .validate(&format!("{cwd}\nkrust> "), &mut 0)
                    .is_ok()
            );
            assert!(group[2].validate("/wrong\nkrust> ", &mut 0).is_err());
        }
    }
}

#[test]
fn shell_assertions_reject_corrupted_responses() {
    let checks = [
        ("clear", Check::Clear, "\x1b[2J\x1b[H", ""),
        (
            "env",
            Check::Environment("KEY=value".into()),
            "KEY=value",
            "KEY=value\nKEY=stale",
        ),
        (
            "mem",
            Check::Patterns(vec![
                r"(?m)^MemTotal: +[1-9][0-9]* kB$",
                r"(?m)^MemFree: +[0-9]+ kB$",
            ]),
            "MemTotal: 42 kB\nMemFree: 0 kB",
            "MemTotal: 0 kB\nMemFree: 0 kB",
        ),
        (
            "ls",
            Check::Listing,
            "  note.txt\n  .hidden",
            "  note.txt\n  .hidden\n  ./",
        ),
        (
            "stat",
            Check::Patterns(vec![
                r"^note.txt +perms=[rwx-]{9} ino=[1-9][0-9]* +size=20 +type=file$",
            ]),
            "note.txt perms=rw-r--r-- ino=2 size=20 type=file",
            "note.txt perms=rw-r--r-- ino=2 size=19 type=file",
        ),
        (
            "cat",
            Check::Exact("shell round trip two".into()),
            "shell round trip two",
            "shell round trip one",
        ),
        (
            "cd missing",
            Check::Exact("cd: errno -2".into()),
            "cd: errno -2",
            "cd: errno -20",
        ),
    ];
    for (command, check, good, bad) in checks {
        let case = Case::new(command, check);
        assert!(
            case.validate(&format!("{good}\nkrust> "), &mut 0).is_ok(),
            "{command}"
        );
        assert!(
            case.validate(&format!("{bad}\nkrust> "), &mut 0).is_err(),
            "{command}"
        );
    }
    let uptime = Case::new("uptime", Check::Uptime);
    let mut ticks = 0;
    assert!(uptime.validate("ticks since boot: 10", &mut ticks).is_ok());
    assert!(uptime.validate("ticks since boot: 11", &mut ticks).is_ok());
    assert!(uptime.validate("ticks since boot: 9", &mut ticks).is_err());
    assert!(uptime.validate("ticks since boot: 0", &mut ticks).is_err());
}

#[test]
fn mmap_probe_requires_real_boot_mapping() {
    let trace = "virtual_address_offset: 0x10000000000\nHandling Segment: type_: Ok(Load), virtual_addr: 1234,";
    assert_eq!(kernel_address(trace).unwrap(), 0x10000001000);
    for bad in [
        "",
        "virtual_address_offset: 0x1",
        "virtual_address_offset: 0x0\nHandling Segment: type_: Ok(Load), virtual_addr: 0,",
        "virtual_address_offset: 0x800000000000\nHandling Segment: type_: Ok(Load), virtual_addr: 1234,",
    ] {
        assert!(kernel_address(bad).is_err(), "{bad}");
    }
}

#[test]
fn mmap_protection_validators() {
    for (mode, flags) in PROTECTION_MODES {
        let output = format!(
            "KRUST_PROTECTION_ARMED {mode}\n/bin/krust-mmap-protection: exited with status 139"
        );
        let trace = format!(
            "ERROR: EXCEPTION: PAGE FAULT\nERROR: Error Code: PageFaultErrorCode({flags})\nERROR: page fault in user task pid=4 - terminating task\n"
        );
        assert!(protection_response(mode, flags, &output, &trace).is_ok());
        for bad in [
            trace.replace(flags, "WRONG"),
            trace.repeat(2),
            trace.replace("USER_MODE", ""),
            trace.replace("page fault in user task", "page fault in kernel task"),
            format!("{trace}ERROR: errno -1"),
            format!("{trace}ERROR: EXCEPTION: GENERAL PROTECTION FAULT"),
        ] {
            assert!(
                protection_response(mode, flags, &output, &bad).is_err(),
                "{mode}: {bad}"
            );
        }
        for bad in [
            output.replace("139", "127"),
            output.replace("ARMED", "NOT_ARMED"),
            format!("{output}\nKRUST_PROTECTION_FAIL"),
            format!("{output}\nerrno -1"),
            format!("{output}\n{output}"),
        ] {
            assert!(protection_response(mode, flags, &bad, &trace).is_err());
        }
    }
}

fn sample(utility: &support::utilities::Utility) -> &str {
    match utility.validation {
        Validation::Exact(expected) => expected,
        Validation::Listing => "input repeat one bytes strings data.gz data.tar empty",
        Validation::Processes => "PID USER COMMAND\n1 root /bin/busybox",
        Validation::Pattern(_) => match utility.name.as_str() {
            "wc" => " 3 /fixture/input",
            "date" => "123456",
            "od" => " 41 42",
            _ => panic!("missing sample"),
        },
        Validation::Network => panic!("wget requires HTTP fixture"),
    }
}

#[test]
fn utility_validators_and_completion_markers() {
    for name in [
        "sh", "ash", "bash", "ls", "cat", "echo", "cp", "mv", "rm", "mkdir", "rmdir", "ln",
        "touch", "grep", "sed", "awk", "cut", "sort", "uniq", "head", "tail", "wc", "find",
        "xargs", "stat", "chmod", "chown", "ps", "kill", "env", "printenv", "date", "sleep",
        "true", "false", "hexdump", "od", "strings", "gzip", "gunzip", "tar",
    ] {
        let utility = case(name).unwrap();
        let good = sample(&utility);
        assert!(utility.validate(good, utility.status).is_ok(), "{name}");
        assert!(utility.validate(good, 99).is_err(), "{name}");
        assert!(
            utility.validate("INVALID", utility.status).is_err(),
            "{name}"
        );
        let response = format!("{good}\n\nKRUST_UTILITY_DONE:{name}:{}", utility.status);
        assert_eq!(
            utility.response(&response).unwrap(),
            (good.to_string(), utility.status)
        );
        for bad in [
            "missing".into(),
            response.repeat(2),
            response.replace("DONE:", "DONE:wrong"),
            response.replace(
                &format!(":{name}:{}", utility.status),
                &format!(":{name}:invalid"),
            ),
        ] {
            assert!(utility.response(&bad).is_err(), "{name}: {bad}");
        }
    }
    assert!(case("notanapplet").is_err());
    assert!(case("../cat").is_err());
    assert!(
        case("wget")
            .unwrap()
            .validate("", 0)
            .unwrap_err()
            .contains("HTTP fixture")
    );
}

#[test]
#[ignore = "requires an existing native BusyBox binary; no QEMU"]
fn installed_utility_validators() {
    let work = Workspace::new("installed-validators");
    for name in support::utilities::installed(&work, "ext4") {
        let utility = case(&name).unwrap();
        assert!(utility.validate(sample(&utility), utility.status).is_ok());
        assert!(utility.validate("INVALID", utility.status).is_err());
        assert!(utility.validate(sample(&utility), 99).is_err());
    }
}
