# Testing

[Project overview](../../Readme.md) | [Development](../development.md) |
[BusyBox compatibility](../busybox.md)

Run all commands from the repository root. Use the host package, not
`--workspace` (the kernel is a bare-metal binary).

## Cargo targets and filtering

```bash
# Default: host tests, mock harnesses, and guest ELF layout checks.
# No QEMU boots or BusyBox download/build.
cargo test -Z bindeps --locked -p krust
cargo test -Z bindeps --locked -p krust --test pci
cargo test -Z bindeps --locked -p krust --test mmap protection
cargo test -Z bindeps --locked -p krust --test memfs

# Discover the individually named guest cases without running them.
cargo test -Z bindeps --locked -p krust --test busybox -- --list

# Exactly one utility, filesystem and firmware.
cargo test -Z bindeps --locked -p krust --test busybox fat::bios::ls -- --ignored --exact --nocapture

# cat under all four filesystem/firmware combinations.
cargo test -Z bindeps --locked -p krust --test busybox ::cat -- --ignored

# Fork/exec/wait + interactive shell regression, both firmware modes.
cargo test -Z bindeps --locked -p krust --test qemu shell_process -- --ignored --exact --nocapture

# Bounded tmpfs syscalls, sparse files and reclamation on BIOS/UEFI.
cargo test -Z bindeps --locked -p krust --test qemu tmpfs -- --ignored --exact --nocapture

# Efficient full installed-utility matrices (shared boots within each matrix).
cargo test -Z bindeps --locked -p krust --test qemu busybox_ -- --ignored --nocapture

# Exactly one full filesystem/firmware matrix.
cargo test -Z bindeps --locked -p krust --test qemu busybox_ext4_uefi -- --ignored --exact --nocapture

# All QEMU suites, including mounts, built-ins, userspace, CLOEXEC and mmap.
cargo test -Z bindeps --locked -p krust --test qemu -- --ignored --nocapture

# Host-only installed-applet validators, requiring an existing BusyBox.
cargo test -Z bindeps --locked -p krust --test harnesses installed_utility_validators -- --ignored --exact
```

`cloexec`, `mmap`, `memfs`, and `pci` compile the Rust test sources directly as
native Cargo targets; their individual assertions are not hidden behind a shell
wrapper. Host integration tests live directly under `tests/`, with shared helpers
and compile-only fixtures under `tests/support/`. Cargo discovers `cloexec`,
`mmap`, and `memfs` by filename without explicit `[[test]]` entries.

`harnesses` directly tests response validators and a Cargo-built Rust mock runner
from `tests/mock`. It covers fragmented prompts, input echo, bounded output and
exit waits, shutdown diagnostics, process-group cleanup, unchanged-disk guards,
shell state/content assertions, utility completion markers, and exact fault
validation without QEMU. The freestanding Rust guest
fixtures live in the workspace's `tests/guest` package. An artifact dev-dependency
builds six static `no_std`/`no_main` x86_64 executables: tmpfs, both CLOEXEC
images, mmap ranges, mmap protections, and process/fork/exec/wait.
The default `qemu::guest_artifacts_are_static_user_executables` test checks their
ELF layout, user-space entry points, segment permissions, and lack of a dynamic
loader without booting QEMU.

Actual guest execution remains in named, ignored-by-default Cargo integration
tests: `tmpfs`, `cloexec`, `mmap` (including protections), and `shell_process`
(including the process fixture). The shared Rust harness in `tests/support/qemu.rs`
launches the runner, waits for serial prompts, applies deadlines, retains logs,
and cleans up owned process groups. Assertions, fixture creation, and guest
orchestration are Rust, not Bash wrappers. They use the exact Cargo guest
artifacts and built runner
(`CARGO_BIN_EXE_krust`), including with `--release` or a custom Cargo target dir.
Individual suites can have additional constraints, such as CLOEXEC's debug-only
kernel shell command.

Cargo tests are the only test entry points; there are no shell test wrappers.
The only remaining shell scripts build BusyBox fixtures and install its applets.
Tests invoke applet installation and external image/archive tools, not nested
Cargo. Guest syscall/startup/runtime support is
shared across the Rust binaries; fault tests use explicit assembly for
intentionally forbidden accesses. No host C compiler or libc is needed.

The `busybox` target has 160 independently selectable cases for the current 40
installed utilities. `true` and `false` are named `true_exit_status` and
`false_exit_status`. The `qemu` matrix tests still discover **all** installed
applets dynamically; use these for complete coverage and faster full runs.
Standard Cargo substring filters, `--exact`, and `--skip` apply. Missing fixtures,
disabled requested applets, and real compatibility failures fail explicitly;
known gaps are not marked as expected successes.

## Fixtures, isolation and logs

Set `KRUST_TEST_BUSYBOX` to override `target/rootfs/bin/busybox`, or
`KRUST_TEST_DISK` to override the existing ext4 VHD used by disk-based suites.
QEMU tests require their existing host tools (QEMU/firmware, and, depending on
the suite, ext4 tooling); Cargo does not download BusyBox or
create the external disk fixture. Each test claims a unique workspace and uses
QEMU snapshots. Serial output, kernel logs, and utility reports do not share
filenames between tests, so there is no global script lock. Limit parallel boots
with `--test-threads=2` on resource-constrained hosts.

Logs are retained under unique `target/cargo-tests/native/<suite>-<id>/`
directories; the exact path is printed when each test starts. Each boot has
serial and kernel logs; utility runs also retain per-case logs, syscall inventories,
and `report.tsv`. Scratch images are removed on normal completion, assertion
failure, or deadline; the runner and its children are isolated in an owned process
group and the runner is reaped. This is RAII cleanup, not a guarantee for abrupt
test-process termination such as SIGKILL. Source and snapshot-base disks are
hash-checked even on assertion failure. `KRUST_TEST_OUTPUT_DIR` overrides the
log parent. `KRUST_TEST_RUNNER` optionally overrides Cargo's built runner.
Use Cargo filters for
[focused utility runs](../busybox.md#installed-utility-compatibility-matrix).

## Detailed suites

| Guide | Coverage |
|-------|----------|
| [Shell and userspace](shell.md) | Kernel built-ins, BusyBox process/interactive regressions, syscall tracing, mock harnesses |
| [BusyBox compatibility](../busybox.md) | Installed applet matrix, FAT wrappers, ext4 symlinks, current process semantics |
| [Storage and boot mounts](../storage.md#mount-regression-tests) | Root selection, fstab validation, mount ordering and failures |
| [RAM filesystems](../storage.md#ram-filesystem-regression-tests) | Shared ramfs/tmpfs core, sparse storage, limits, allocation failures, lifecycle and syscall regressions |
| [Descriptors](descriptors.md) | Descriptor-local CLOEXEC, duplication flags, fork/exec retention |
| [Memory](memory.md) | mmap bounds, supervisor overlap, permissions, expected user faults, ARM64 compile checks |
| [PCI](../pci.md) | Synthetic PCI configuration tests and a bounded QEMU inspection |
