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
bash scripts/tests/test-utility-cases.sh
scripts/test-busybox-utilities.sh
scripts/test-busybox-utilities.sh target/rootfs/bin/busybox target/debug/krust ext4
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

By default both BIOS and UEFI run the complete matrix. A hung/crashed case fails
after a bounded deadline and the next case boots a fresh snapshot, so one broken
utility cannot prevent the remaining utilities from being attempted. The source
disk is hash-checked and temporary images/processes are cleaned up.

Results, including trace-verified applet launches, are in
`target/busybox-utilities-{fat,ext4}.tsv`; per-case console/kernel
logs and per-firmware syscall/unimplemented-syscall inventories share that
prefix. Any functional failure, missing completion marker, exception, timeout,
or bad shutdown makes the suite exit nonzero.

These are compatibility tests, not a claim that all utilities or Linux syscalls
already work. Missing filesystem mutations, descriptor duplication, vectored
reads and other syscall gaps are intentionally reported as failures, not
skipped or accepted as successful help output. The currently disabled `wget`
applet requires a guest-reachable HTTP fixture before it can pass if enabled.
The host-only validator checks positive/negative output and status assertions;
it does not replace the real QEMU runs.

See [Cargo test targets and filtering](testing/README.md) for individual guest
cases, full matrix targets, fixture overrides, and report directories. The
script also supports focused runs:

```bash
scripts/test-busybox-utilities.sh --firmware uefi --utility cat --utility false --output-dir target/selected-utilities
```
