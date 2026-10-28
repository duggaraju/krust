# BusyBox and userspace

[Project overview](../Readme.md) | [Storage](storage.md) |
[Cargo tests](testing/README.md) | [Shell regressions](testing/shell.md)

Run all commands from the repository root. See [development prerequisites](development.md)
for BusyBox build tools and the optional `--libc glibc` setup flag.

## FAT and ext4 applet installation

FAT has no symlinks: its `sh` and `ash` entrypoints are executable BusyBox
copies, while other enabled applets use small scripts with `#!/bin/busybox sh`.
Only compiled applets are installed; no fake `bash` alias is generated. Extra
copies of the shell binary are the FAT space tradeoff. Ext4 uses symlinks.

Existing FAT images must be regenerated to replace their recursive `/bin/sh`
wrapper. Until then, use `/bin/busybox sh` directly. Reinstall applets without
rebuilding BusyBox, then create a fresh image:

```bash
scripts/install-busybox-applets.sh target/busybox-build/busybox target/rootfs fat
cargo run -Z bindeps -- mkdisk target/rootfs target/root-fat.vhd --format fat
```

The root layout must contain `bin/busybox`, not an extra rootfs directory.
See [root selection and fstab](storage.md#root-selection-and-etcfstab) for mounts.

## Process and interpreter behavior

The kernel permits up to four interpreter redirects, then rejects the exec with
`ELOOP` rather than looping forever. This also protects against old recursive
scripts. Errors when launching from the kernel shell are logged with their
errno and the child exits with status 127; the parent shell remains responsive.

Userspace fork resumes the child from its saved syscall register frame
with return value zero, rather than a copied Rust kernel continuation. User
page-table branches are private to each address space, so child writes and
exec cannot replace the parent's mappings. Exec copies bounded argv/envp
vectors from validated user pages before replacing the image, preserving empty
arguments and an explicitly empty environment.

Wait status uses Linux's four-byte `int` buffer and exit-code encoding.
Any-child/positive-PID waits support `WNOHANG` and return `ECHILD` after the last
child is reaped. Process-group selection, nonzero rusage, and unsupported wait
options are explicitly rejected. Full signal/wait semantics and per-task
x87/SSE state remain separate work items (#14/#15/#50).

See the [shell/process regression](testing/shell.md#busybox-shell-and-process-regression)
for BIOS/UEFI fork, exec, wait, fault-recovery, and interactive shell coverage.

## Installed-utility compatibility matrix

To exercise **every installed `/bin` utility through the userspace BusyBox
shell**, rather than similarly named kernel built-ins:

```bash
# Host-only validator checks using an existing BusyBox binary.
cargo test -Z bindeps --locked -p krust --test harnesses installed_utility_validators -- --ignored --exact

# All four full matrices: FAT/ext4, BIOS/UEFI.
cargo test -Z bindeps --locked -p krust --test qemu busybox_ -- --ignored --nocapture --test-threads=2

# One complete matrix, sharing a guest boot between utilities.
cargo test -Z bindeps --locked -p krust --test qemu busybox_ext4_uefi -- --ignored --exact --nocapture
```

The matrix is derived from the applets actually installed by the rootfs setup
(40 with the current BusyBox), not every applet compiled into BusyBox. Each case
runs `/bin/busybox sh /cases/<utility>` and invokes the installed `/bin/<utility>`
path with functional input/output and exit-status checks. This covers FAT
wrappers and, with `ext4`, symlinks. `false` must return 1; success-shaped stubs
for mutations are insufficient because destination contents, metadata, or
existence are checked too. New-file destinations generally use a RAM-backed
`/scratch`; removal, metadata and compression cases use owned disk fixtures.
Compression and `xargs -a` avoid shell redirection so a broken redirection
syscall cannot prevent those applets from actually launching.

Guest tests are ignored by default; `--ignored` explicitly enables them. The
`busybox_` filter above runs both filesystems under BIOS and UEFI. A hung/crashed case fails
after a bounded deadline and the next case boots a fresh snapshot, so one broken
utility cannot prevent the remaining utilities from being attempted. The source
disk is hash-checked and temporary images/processes are cleaned up.

Results, including trace-verified applet launches, are in
`report.tsv` in the unique `target/cargo-tests/native/busybox-*` directory
printed by the Rust harness. Per-case response/kernel logs, boot console logs,
and syscall/unimplemented-syscall inventories are retained alongside it.
Any functional failure, missing completion marker, exception, timeout,
or bad shutdown makes the suite exit nonzero.

These are compatibility tests, not a claim that all utilities or Linux syscalls
already work. Missing filesystem mutations, descriptor duplication, vectored
reads and other syscall gaps are intentionally reported as failures, not
skipped or accepted as successful help output. The currently disabled `wget`
applet requires a guest-reachable HTTP fixture before it can pass if enabled.
The host-only validator checks positive/negative output and status assertions;
it does not replace the real QEMU runs.

The native `busybox` target has 160 independently selectable tests for the
current 40 installed utilities. Each selected test creates its own image and
boot; use the `qemu` matrix targets above for efficient complete coverage.
Cargo filters select utility, filesystem, and firmware:

```bash
# Discover available test names without booting QEMU.
cargo test -Z bindeps --locked -p krust --test busybox -- --list

# A single utility under one filesystem/firmware combination.
cargo test -Z bindeps --locked -p krust --test busybox ext4::uefi::cat -- --ignored --exact --nocapture

# cat under all four filesystem/firmware combinations.
cargo test -Z bindeps --locked -p krust --test busybox ::cat -- --ignored --nocapture --test-threads=2

# Multiple exact test names go after --.
cargo test -Z bindeps --locked -p krust --test busybox -- --ignored --exact --nocapture ext4::uefi::cat ext4::uefi::false_exit_status
```

`true` and `false` use the test names `true_exit_status` and `false_exit_status`.
Set `KRUST_TEST_BUSYBOX` to override the BusyBox fixture and
`KRUST_TEST_OUTPUT_DIR` to change the parent of the unique log/report directories.
See [Cargo test targets and filtering](testing/README.md) for other guest suites,
disk fixture overrides, and isolation details.
