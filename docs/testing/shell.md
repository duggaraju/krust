# Shell and userspace regressions

[Testing index](README.md) | [BusyBox](../busybox.md) |
[Project overview](../../Readme.md)

Run all commands from the repository root.

## BusyBox shell and process regression

```bash
scripts/test-fat-shell.sh target/rootfs/bin/busybox
```

The same suite is the `shell_process` Cargo test; see
[Cargo filtering](README.md#cargo-targets-and-filtering).

It verifies ext4-to-FAT applet installation, real `sh`/`ash` execution, repeated
`ls` from automated and interactive BusyBox shells, valid one/four-level script
loading, self/two-script loops, over-limit chains, and parent-shell recovery on
BIOS and UEFI. A freestanding C fixture additionally checks preserved fork
registers and stack/global/mmap isolation across child exec, exact argv/envp,
invalid pointers, oversized strings, a vector crossing a page boundary,
failed-exec recovery, a canary next to the wait status buffer, and parent
recovery after a fork child deliberately faults. Exactly that one user-mode
page fault is required; any additional exception fails the regression. The test
requires a host C compiler and uses an owned temporary image, QEMU snapshots,
and unchanged-image hash checks.

## BIOS/UEFI kernel-shell commands

```bash
cargo build -Z bindeps --locked
scripts/test-smoke-userspace.sh
scripts/smoke-shell.sh target/krust-disk-ext4.vhd
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
may shorten it. Logs remain in
`target/shell-smoke-{bios,uefi}-{disk,ramfs}{,-kernel,-response}.log`
(response logs exist only for assertion failures). The existing VHD must contain
`/bin/busybox`; QEMU snapshots leave it unchanged. No kernel behavior is changed.

The separate RAM-backed mutation phase is intentional:
[ext4 mkdir currently returns EIO (#51)](https://github.com/duggaraju/krust/issues/51).
Passing this suite does **not** certify ext4 writes. Listing tests also preserve
the current shell behavior: dotfiles are shown without `-a`, and RAM-backed
`ls -a` does not synthesize `.`/`..`; this is not POSIX `ls` certification.

## BIOS/UEFI userspace smoke

With an existing ext4 root VHD containing `/bin/busybox`, run:

```bash
cargo check -p kernel --target x86_64-unknown-none --locked
cargo build -Z bindeps --locked
scripts/smoke-userspace.sh target/krust-disk-ext4.vhd
```

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
`KRUST_SMOKE_TIMEOUT_SECONDS=3 scripts/smoke-userspace.sh disk.vhd runner`.
Cleanup terminates only the isolated runner process group, allows 0.2 seconds
for graceful teardown, then kills remaining group members. Generated boot
images, ramdisks, and traces are confined to an exclusively created directory
under `target/`, removed on success, failure, timeout, or caught INT/TERM.
The source VHD hash is checked on every exit after its initial hash, including
failures; a changed/unreadable VHD fails even an otherwise successful suite.
Serial logs and copied kernel traces remain in
`target/userspace-smoke-{bios,uefi}*.log` (overwritten per mode invocation);
assertion failures also preserve the normalized `*-response.log`.

## Syscall tracing and console selection

Syscall tracing defaults to `false`. Enable it at build time for both ordinary
boot images and regenerated BIOS/UEFI serial-shell ramdisks:

```bash
KRUST_BOOT_SYSCALL_TRACE=true cargo build -Z bindeps --locked
scripts/smoke-userspace.sh target/krust-disk-ext4.vhd
grep -E 'syscall (entry: pid=|exit: pid=|meta: name=)' target/userspace-smoke-{bios,uefi}-kernel.log
```

These are actual syscall dispatch records, not just the boot configuration.
The smoke accepts structured `syscall failed: ... errno=...` trace diagnostics
for userspace probes/retries; command failures and other kernel errors still fail.
`--headless` and `--pty` select shell port 2; BIOS with only raw
`-display none`/`--display none` selects port 1. Ordinary boot console settings
are unchanged. Rebuild without `KRUST_BOOT_SYSCALL_TRACE` (or set it to `false`)
to disable these records; rerunning the smoke overwrites its logs.

## Host-only harness checks

Requirements: Bash, GNU `timeout`, `realpath`, `sha256sum`, util-linux `setsid`,
and the normal QEMU/build prerequisites for real guest runs.

```bash
scripts/test-smoke-userspace.sh
```

The controlled fake runner requires no QEMU or real VHD. It checks split prompts
and rejects premature/batched commands. Tests cover success, missing shell,
command failure, boot/mid-command and closed-output timeouts, echoed markers
without output, missing pwd/proc content, expected syscall-probe traces versus
unrelated kernel errors, runner failure, invalid deadlines/paths/arguments, and
disk mutation on failure and success. They verify owned child processes are
reaped, scratch directories removed, and an unrelated process left alive.
Each case uses a three-second deadline; case logs remain in
`target/userspace-smoke-test-*.log`.

The 33 host scenarios include 11 shell scenarios (success plus injected
state/content/diagnostic/clear/uptime/shutdown failures and timeout); these also
check bounded execution and cleanup. The four real shell boots, not those
controlled mocks, validate the kernel.

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
