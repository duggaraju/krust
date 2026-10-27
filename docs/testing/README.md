# Testing

[Project overview](../../Readme.md) | [Development](../development.md) |
[BusyBox compatibility](../busybox.md)

Run all commands from the repository root. Use the host package, not
`--workspace` (the kernel is a bare-metal binary).

## Cargo targets and filtering

```bash
# Default: host unit tests, native PCI/mmap/CLOEXEC tests and mock-harness checks.
# No QEMU boots or BusyBox download/build.
cargo test -Z bindeps --locked -p krust
cargo test -Z bindeps --locked -p krust --test pci
cargo test -Z bindeps --locked -p krust --test mmap protection

# Discover the individually named guest cases without running them.
cargo test -Z bindeps --locked -p krust --test busybox -- --list

# Exactly one utility, filesystem and firmware.
cargo test -Z bindeps --locked -p krust --test busybox fat::bios::ls -- --ignored --exact --nocapture

# cat under all four filesystem/firmware combinations.
cargo test -Z bindeps --locked -p krust --test busybox ::cat -- --ignored

# Fork/exec/wait + interactive shell regression, both firmware modes.
cargo test -Z bindeps --locked -p krust --test qemu shell_process -- --ignored --exact --nocapture

# Efficient full installed-utility matrices (shared boots within each matrix).
cargo test -Z bindeps --locked -p krust --test qemu busybox_ -- --ignored --nocapture

# All QEMU suites, including mounts, built-ins, userspace, CLOEXEC and mmap.
cargo test -Z bindeps --locked -p krust --test qemu -- --ignored --nocapture

# Host-only installed-applet validators, requiring an existing BusyBox.
cargo test -Z bindeps --locked -p krust --test harnesses installed_utility_validators -- --ignored --exact
```

`cloexec`, `mmap`, and `pci` compile the existing Rust test sources directly as
native Cargo targets; their individual assertions are not hidden behind a shell
wrapper. `harnesses` runs the existing mock-runner scripts. QEMU orchestration and
the freestanding C guest fixture remain in their existing scripts, exposed as
named, ignored-by-default Cargo integration tests. They use Cargo's built runner
(`CARGO_BIN_EXE_krust`), including with `--release` or a custom Cargo target dir.

The `busybox` target has 160 independently selectable cases for the current 40
installed utilities. `true` and `false` are named `true_exit_status` and
`false_exit_status`. The `qemu` matrix tests still discover **all** installed
applets dynamically; use these for complete coverage and faster full runs.
Standard Cargo substring filters, `--exact`, and `--skip` apply. Missing fixtures,
disabled requested applets, and real compatibility failures fail explicitly;
known gaps are not marked as expected successes.

## Fixtures, serialization and logs

Set `KRUST_TEST_BUSYBOX` to override `target/rootfs/bin/busybox`, or
`KRUST_TEST_DISK` to override the existing ext4 VHD used by disk-based suites.
QEMU tests require their existing host tools (QEMU/firmware, and, depending on
the suite, a C compiler and ext4 tooling); Cargo does not download BusyBox or
create the external disk fixture. Script execution is serialized within each
test target because legacy suites share log names. A failed test does not stop
subsequent cases or poison their execution lock. Do not run overlapping suite
processes against those shared legacy logs.

Individual BusyBox logs/reports are retained under
`target/cargo-tests/busybox/<filesystem>/<firmware>/<utility>/`; full matrix
reports are under `target/cargo-tests/qemu/<filesystem>/<firmware>/`. Other suites
retain their existing log paths. The original CLI also supports
[focused utility runs](../busybox.md#installed-utility-compatibility-matrix).

## Detailed suites

| Guide | Coverage |
|-------|----------|
| [Shell and userspace](shell.md) | Kernel built-ins, BusyBox process/interactive regressions, syscall tracing, mock harnesses |
| [BusyBox compatibility](../busybox.md) | Installed applet matrix, FAT wrappers, ext4 symlinks, current process semantics |
| [Storage and boot mounts](../storage.md#mount-regression-tests) | Root selection, fstab validation, mount ordering and failures |
| [Descriptors](descriptors.md) | Descriptor-local CLOEXEC, duplication flags, fork/exec retention |
| [Memory](memory.md) | mmap bounds, supervisor overlap, permissions, expected user faults, ARM64 compile checks |
| [PCI](../pci.md) | Synthetic PCI configuration tests and a bounded QEMU inspection |
