mod support;

#[test]
fn userspace_smoke_validators() {
    support::run("scripts/test-smoke-userspace.sh", |_| {});
}

#[test]
fn mmap_protection_validators() {
    support::run("scripts/test-mmap-protection-harness.sh", |_| {});
}

#[test]
fn failed_script_does_not_poison_execution_lock() {
    let failure = std::panic::catch_unwind(|| {
        support::run("-c", |command| {
            command.arg("exit 7");
        });
    });
    assert!(failure.is_err());
    support::run("-c", |command| {
        command.arg("exit 0");
    });
}

#[test]
#[ignore = "requires an existing native BusyBox binary; no QEMU"]
fn installed_utility_validators() {
    support::run("scripts/tests/test-utility-cases.sh", |command| {
        command.arg(support::fixture(
            "KRUST_TEST_BUSYBOX",
            "target/rootfs/bin/busybox",
        ));
    });
}

#[test]
#[ignore = "requires an existing native BusyBox binary; no QEMU"]
fn utility_selection_rejects_uninstalled_applet() {
    let output = std::process::Command::new("bash")
        .arg("scripts/test-busybox-utilities.sh")
        .arg(support::fixture(
            "KRUST_TEST_BUSYBOX",
            "target/rootfs/bin/busybox",
        ))
        .arg(env!("CARGO_BIN_EXE_krust"))
        .args([
            "fat",
            "--utility",
            "notanapplet",
            "--output-dir",
            "target/cargo-tests/invalid-utility",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("launch utility selector");
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Utility is not installed: notanapplet"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn utility_selection_rejects_invalid_arguments() {
    for arguments in [
        vec!["--firmware", "invalid"],
        vec!["--firmware"],
        vec!["--utility", "../cat"],
        vec!["--utility"],
        vec!["--utility", "cat", "--utility", "cat"],
        vec!["--output-dir"],
        vec!["--output-dir", "--help"],
        vec!["--unknown"],
        vec!["unused", "unused", "invalid-format"],
        vec!["a", "b", "fat", "extra"],
    ] {
        let output = std::process::Command::new("bash")
            .arg("scripts/test-busybox-utilities.sh")
            .args(&arguments)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("launch utility selector");
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("Usage:"),
            "{arguments:?}: missing actionable error"
        );
    }
}
