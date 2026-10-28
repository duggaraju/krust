# Shell and userspace regressions

[Testing index](README.md) | [BusyBox](../busybox.md) |
[Project overview](../../Readme.md)

Run all commands from the repository root.

## Init selection and PID 1

```bash
cargo test -Z bindeps --locked -p krust --test qemu init_process -- --ignored --exact --nocapture
```

The init fixture checks PID 1, parent PID 0, process group, cwd, initial argv,
environment, and inherited console output. FAT/shebang and ext4/symlink init
selection and explicit `init=` overrides run on BIOS and UEFI. The suite also
checks missing/invalid explicit init failures, PID 1 exit/fault failures, and
the kernel-shell PID 1 fallback when `/sbin/init` is absent. All images are
owned test fixtures; there is no placeholder supervisor task.

The real BusyBox init-wrapper regression boots FAT and ext4 fixtures on both
firmware modes, checks `$$` is 1, runs an external applet, and verifies that
exiting PID 1 stops the kernel with failure. Syscall tracing stays disabled so
an idle serial console cannot accidentally appear as EOF between commands.

```bash
cargo test -Z bindeps --locked -p krust --test qemu init_shell -- --ignored --exact --nocapture
KRUST_TEST_BUSYBOX=target/rootfs-glibc/bin/busybox \
  cargo test -Z bindeps --locked -p krust --test qemu init_shell -- --ignored --exact --nocapture
```

Serial and virtual-console file reads wait for input; internal hardware polling
stays nonblocking. Empty console reads use the existing interrupt/HLT idle wait,
not a CPU spin loop. The PIT currently wakes idle reads every 10 ms to check for
input, so this is not yet a per-device interrupt-driven scheduler wait queue.
The init probe also checks that zero-length terminal reads still return immediately.

## Terminal echo and idle reads

```bash
cargo test -Z bindeps --locked -p krust --test terminal
cargo test -Z bindeps --locked -p krust --test qemu terminal_console -- --ignored --exact --nocapture
```

The host tests exercise the Linux termios layouts, raw/canonical transitions,
echo suppression/restoration, pending-input flushing, and output translation.
The guest probe checks real serial and virtual-console devices on FAT/ext4 and
BIOS/UEFI: all `TCGETS`/`TCSETS*` and extended variants, invalid descriptors and
user buffers, raw reads before newline, canonical reads through newline, and
single/no echo as configured. Delayed input must wait at least 50 guest ticks,
with CPU usage bounded to one quarter of elapsed ticks plus two rounding ticks.
The BusyBox init-shell test also requires exactly one echo of its typed command.

Terminal settings belong to the device and are shared by aliases such as
`/dev/console`; the shell can disable `ECHO` and `ICANON` while its line editor
is active and restore them afterward. This implements the existing line
discipline flags, not complete termios support (for example, baud changes and
`VMIN`/`VTIME` timeout semantics are not implemented).

## BusyBox shell and process regression

```bash
cargo test -Z bindeps --locked -p krust --test qemu shell_process -- --ignored --exact --nocapture
```

This is the `shell_process` Cargo test; see
[Cargo filtering](README.md#cargo-targets-and-filtering).

It verifies ext4-to-FAT applet installation, real `sh`/`ash` execution, repeated
`ls` from automated and interactive BusyBox shells, valid one/four-level script
loading, self/two-script loops, over-limit chains, and parent-shell recovery on
BIOS and UEFI. A Cargo-built, freestanding Rust fixture additionally checks preserved fork
registers and stack/global/mmap isolation across child exec, exact argv/envp,
invalid pointers, oversized strings, a vector crossing a page boundary,
failed-exec recovery, a canary next to the wait status buffer, and parent
recovery after a fork child deliberately faults. It also checks the 52-field
Linux proc stat layout, NUL-separated exec arguments, and `WUNTRACED` waits.
Startup checks require a 16-byte-aligned stack, contiguous auxiliary vectors
with empty/nonempty environments, a NULL static-executable finalizer, and
`ENOSYS` for unsupported `rseq`. The fixture exercises fork-style `clone`,
including a child-TID write crossing a page boundary, parent-memory isolation,
and rejection of unsupported clone modes and invalid TID pointers. Relative
`*at` calls are tested with both 32-bit representations of `AT_FDCWD`.
The interactive shell runs userspace `ps`, and the kernel process listing must
return to its pre-test baseline after shell exit, with no leaked zombies.
CPU-accounting checks require both user and kernel counters to advance by at
least two `USER_HZ` ticks under their respective workloads. They also verify
fork resets counters, exec preserves usage, the parent is not charged for child
execution, zombie totals stay frozen while the parent runs, and wait collects
child CPU time. `KRUST_CPU_ACCOUNTING_OK` is required before the process marker.
Exactly that one user-mode
page fault is required; any additional exception fails the regression. The test
uses an owned temporary image, QEMU snapshots, and unchanged-image hash checks.
The fixture is built from [the guest package](../../tests/guest/Cargo.toml)
using the pinned Rust toolchain; no host C compiler or libc is needed.

Deterministic host tests cover nested mode transitions, repeated snapshots,
idle/interrupt exclusion, resumed execution, sub-tick accumulation, descendant
totals, and counter saturation without relying on QEMU timing:

```bash
cargo test -Z bindeps --locked -p krust --test accounting
```

## BIOS/UEFI kernel-shell commands

```bash
cargo test -Z bindeps --locked -p krust --test harnesses
cargo test -Z bindeps --locked -p krust --test qemu shell_commands -- --ignored --exact --nocapture
```

This is a real QEMU **integration** suite for the kernel's built-ins, not
BusyBox replacements or host-only unit tests. It reuses the serial prompt,
deadline, isolated-process cleanup, and source-disk hash checks.
Each firmware mode runs two fresh boots:

- **Disk boot (33 cases):** help, echo/empty input, both environment-setting
  forms and updates, clear-screen controls, memory/process/device/PCI/module
  listings, monotonic uptime, exact missing-path diagnostics and recovery,
  then a BusyBox child to verify external execution still works.
- **RAM-backed boot (77 cases, `--no-vhd`):** mkdir, relative/absolute navigation,
  write/read/overwrite round trips, directory and long listings, stat metadata,
  invalid arguments, missing paths, non-directory navigation, and unknown
  commands. Every intentional failure is followed by a healthy echo and a
  cwd assertion. BIOS uses `shutdown`; UEFI uses its `exit` alias.

Assertions are scoped to each response and reject echoed input, wrong/missing
diagnostics, lost environment/cwd state, corrupt content, and unexpected errors.
The 120-second deadline applies **per boot**; `KRUST_SMOKE_TIMEOUT_SECONDS`
may shorten it. Serial and kernel logs remain in the unique `shell-*` directory
printed by the Rust harness under `target/cargo-tests/native/`.
The existing VHD must contain
`/bin/busybox`; QEMU snapshots leave it unchanged. No kernel behavior is changed.

The separate RAM-backed mutation phase is intentional:
[ext4 mkdir currently returns EIO (#51)](https://github.com/duggaraju/krust/issues/51).
Passing this suite does **not** certify ext4 writes. Listing tests also preserve
the current shell behavior: dotfiles are shown without `-a`, and RAM-backed
`ls -a` does not synthesize `.`/`..`; this is not POSIX `ls` certification.

## BIOS/UEFI userspace smoke

With an existing ext4 root VHD containing `/bin/busybox`, run:

```bash
cargo test -Z bindeps --locked -p krust --test qemu userspace_smoke -- --ignored --exact --nocapture
```

The default disk is `target/krust-disk-ext4.vhd`; set `KRUST_TEST_DISK` to use
another existing image. Cargo builds the runner and guest fixtures automatically.

The probe waits for each serial kernel-shell prompt (rather than sleeping
through boot), changes to `/`, and runs BusyBox `pwd`, `echo`, `ls /proc /dev`,
`cat /proc/meminfo`, `cat /proc/cpuinfo`, `true`, and another `echo` directly
through the kernel shell's fork/wait/exec path before requesting debug-exit
shutdown. It tests both BIOS and UEFI with QEMU `-snapshot`; the supplied VHD
must already contain `/bin/busybox`. Per-command assertions check `/`, output
markers, proc/dev listings, memory fields, and CPU fields, without accepting
echoed command strings as output. Nonzero runner/command status, kernel errors,
missing shell/prompts, and stalled output/shutdown fail the probe.

The deadline is 120 seconds **per boot**, including command execution and
shutdown. For short host-side tests, `KRUST_SMOKE_TIMEOUT_SECONDS` accepts only
decimal integers from `1` to `120` (no leading zeros); e.g.
`KRUST_SMOKE_TIMEOUT_SECONDS=3 cargo test -Z bindeps --locked -p krust --test qemu userspace_smoke -- --ignored --exact`.
Cleanup terminates only the isolated runner process group, allows a short grace
period, then kills remaining group members and reaps the runner. Generated boot
images, ramdisks, and traces are confined to an exclusively created directory
under the owned test workspace, removed on success, assertion failure, or timeout.
Abrupt termination of the entire test process does not run Rust destructors.
Kernel-shell suites make an owned ext4 copy with `/sbin/init` removed so that
the kernel-shell fallback, rather than a rootfs's userspace init, is exercised.
The source VHD hash is checked on every exit after its initial hash, including
failures; a changed/unreadable VHD fails even an otherwise successful suite.
Serial logs and copied kernel traces remain in the unique `userspace-*` directory
printed under `target/cargo-tests/native/`; runs never overwrite each other's logs.

## Syscall tracing and console selection

Syscall tracing defaults to `false`. Enable it at build time for both ordinary
boot images and regenerated BIOS/UEFI serial-shell ramdisks:

```bash
KRUST_BOOT_SYSCALL_TRACE=true cargo test -Z bindeps --locked -p krust --test qemu userspace_smoke -- --ignored --exact --nocapture
grep -E 'syscall (entry: pid=|exit: pid=|meta: name=)' target/cargo-tests/native/userspace-*/*-kernel.log
```

These are actual syscall dispatch records, not just the boot configuration.
The smoke accepts structured `syscall failed: ... errno=...` trace diagnostics
for userspace probes/retries; command failures and other kernel errors still fail.
`--headless` and `--pty` select shell port 2; BIOS with only raw
`-display none`/`--display none` selects port 1. Ordinary boot console settings
are unchanged. Rebuild without `KRUST_BOOT_SYSCALL_TRACE` (or set it to `false`)
to disable these records; each smoke run retains a separate set of logs.

## Host-only harness checks

The host harness uses a Cargo-built Rust mock runner, without QEMU or a real VHD.
Disk guards require `sha256sum`.

```bash
cargo test -Z bindeps --locked -p krust --test harnesses
```

Rust tests cover fragmented prompts, echoed markers without output, missing
traces, expected syscall-probe diagnostics versus unrelated kernel errors,
runner failure, invalid deadlines/probe flags, source-disk mutation,
shutdown errors, and EOF while the runner remains alive. They verify bounded
execution, owned-child termination, runner reaping, retained traces, and survival
of an unrelated sentinel. Pure validators also reject stale environment values,
wrong cwd/content/metadata/diagnostics, missing clear controls, decreasing uptime,
bad utility markers/statuses, and mismatched or extra page faults. Controlled
timeout cases use subsecond deadlines. Logs remain under `target/cargo-tests/native/`.

The four real shell boots, not the controlled mocks, validate the kernel.

## Firmware initialization background

The userspace smoke is an entry/IRQ regression, not certification of every
userspace syscall. It deliberately uses the kernel shell's `fork_execve` path.
The userspace fork and argv/envp bugs originally documented as #48/#49 are now
covered by the separate [process regression](#busybox-shell-and-process-regression)
and [current process behavior](../busybox.md#process-and-interpreter-behavior).

The kernel must establish CPU/device state instead of inheriting firmware
defaults. BIOS can leave the legacy PIC's IRQ0 at vector `0x08` (the
double-fault vector), while UEFI masks the PIC. A pending timer interrupt then
enters the double-fault IST before the first userspace instruction; an immutable
IST stack in read-only memory misleadingly produces a supervisor write fault
at the ELF entry. The kernel remaps IRQs to `0x20..0x2f`, enables only the
handled PIT IRQ, acknowledges interrupts, and allocates a writable exception
stack. BIOS also leaves CR4.OSFXSR clear, unlike UEFI, causing BusyBox's first
SSE instruction to fault. Explicit x87/SSE initialization fixes that second
firmware dependency. Kernel page permissions and CR0.WP remain unchanged.
