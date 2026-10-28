# BusyBox and userspace

[Project overview](../Readme.md) | [Storage](storage.md) |
[Cargo tests](testing/README.md) | [Shell regressions](testing/shell.md)

Run all commands from the repository root. See [development prerequisites](development.md)
for BusyBox build tools and the optional `--libc glibc` setup flag.

## Static glibc builds

The setup script supports both musl (default) and the host GCC/glibc toolchain.
Install the glibc development files, including `libc.a`, before building.
Use separate rootfs/image paths to retain an existing musl installation:

```bash
scripts/setup-busybox.sh target/rootfs-glibc target/krust-disk-glibc-ext4.vhd --format ext4 --libc glibc
cargo run -Z bindeps -- run uefi --headless --vhd target/krust-disk-glibc-ext4.vhd
```

The binary must be a static, non-PIE x86-64 ELF executable (`readelf -h` reports
`EXEC`, and `readelf -l` has no `INTERP`). No shared libc or dynamic linker needs
to be installed in the rootfs. From the kernel shell, invoke `/bin/busybox`
with an applet name or `/bin/busybox sh`.

The initial stack keeps argc/argv/envp/auxv contiguous and 16-byte aligned, and
the kernel clears the dynamic-linker finalizer register (`RDX`) on static exec.
Unsupported `rseq` registration returns `ENOSYS` so glibc can disable it.
Relative-path `*at` calls accept both sign-extended and zero-extended 32-bit
`AT_FDCWD` arguments.
The `getpgrp` syscall uses the same process-group state as `getpgid(0)`, allowing
glibc-linked interactive shells to initialize terminal foreground handling.

Glibc's fork path uses `clone`, not the legacy `fork` syscall. Krust supports
fork-style `clone` with `SIGCHLD`, a NULL child stack, and optional
`CLONE_CHILD_SETTID`/`CLONE_CHILD_CLEARTID`. Child TID writes occur only in the
child's private address space; `set_tid_address` updates the exit-clear address,
which is reset on exec and not inherited by plain fork. Unsupported clone
flags, shared-memory/thread creation, and alternate child stacks return
`EOPNOTSUPP`. This does not implement pthreads or futex waiting/waking.

Run the existing BIOS/UEFI smoke with the glibc image:

```bash
KRUST_TEST_DISK=target/krust-disk-glibc-ext4.vhd cargo test -Z bindeps --locked -p krust --test qemu userspace_smoke -- --ignored --exact --nocapture
KRUST_TEST_BUSYBOX=target/rootfs-glibc/bin/busybox cargo test -Z bindeps --locked -p krust --test qemu shell_process -- --ignored --exact --nocapture
```

These are focused compatibility checks, not support for every glibc facility
or BusyBox applet. Signal semantics, per-task FP/SIMD state, and secure entropy
remain incomplete.

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
Any-child/positive-PID waits support `WNOHANG` and accept `WUNTRACED`, including
their combination, so interactive shells can reap exited children. Stopped-child
reporting is not yet implemented. Waits return `ECHILD` after the last child is
reaped. Process-group selection, nonzero rusage, and unsupported wait
options are explicitly rejected. Full signal/wait semantics and per-task
x87/SSE state remain separate work items (#14/#15/#50).

`/proc/<pid>/stat` uses Linux's positional 52-field format with a parenthesized
command name and single-letter state. Untracked accounting fields are zero.
`/proc/<pid>/cmdline` contains NUL-separated arguments, including empty arguments;
kernel tasks without arguments return an empty command line.

### CPU accounting

Each task records user and kernel execution separately in nanoseconds.
Syscall/interrupt mode transitions and scheduler switches settle the running
interval; snapshots include the current interval without double-counting it.
Idle waits are paused, fork starts fresh counters, exec preserves them, and
exit freezes usage. Reaping collects the child's own and already-reaped
descendant usage into the parent's child totals.

Proc stat fields 14–17 expose user, system, child-user, and child-system time
in Linux `USER_HZ` units (100 ticks/second), converting only after accumulation.
The x86 accounting clock uses the TSC with CPUID frequency information or
bounded PIT channel-2 calibration. It assumes a constant-rate TSC and the
current single-running-CPU kernel; non-invariant hardware is warned about.
The ARM64 counter adapter alone does not provide ARM64 runtime accounting.

Remaining reporting and timekeeping work is tracked in:

- [#70](https://github.com/duggaraju/krust/issues/70): Linux `times(2)`.
- [#71](https://github.com/duggaraju/krust/issues/71): `getrusage` and `wait4` usage.
- [#72](https://github.com/duggaraju/krust/issues/72): CPU clocks and Linux time APIs.
- [#73](https://github.com/duggaraju/krust/issues/73): real, accounting-safe `nanosleep`.
- [#74](https://github.com/duggaraju/krust/issues/74): clocksource/SMP/ARM64 hardening.

Those APIs are not implemented by core accounting: in particular, `nanosleep`
is still a stub and `times` still returns raw counter uptime without a `tms`.

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
