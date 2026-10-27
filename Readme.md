# Getting Started.
* Install the repository toolchain from the repository root:
```bash
rustup toolchain install
```

`rust-toolchain.toml` pins `nightly-2026-03-01`, including
`llvm-tools-preview`, `rust-src` (for building the bootloader), and the
`x86_64-unknown-none` and `aarch64-unknown-none` targets. Cargo automatically uses this pin in the
repository. Do not override it with `+nightly`: newer floating nightlies
can break the unstable `Step` trait implementations in the locked `x86_64`
dependencies. Keep dependency versions and `Cargo.lock` unchanged when
setting up the toolchain.

* Install BusyBox rootfs build tools
```bash
sudo apt install build-essential wget libncurses-dev musl-tools e2fsprogs
```

To build BusyBox with glibc instead of musl, pass `--libc glibc` to `scripts/setup-busybox.sh`.

* Build the code
```bash
cargo build -Z bindeps --locked
```

* Check the kernel without building disk images
```bash
cargo check -p kernel --target x86_64-unknown-none --locked
```

## Dev loop (headless QEMU)

### Creating data disks

```bash
cargo run -Z bindeps -- mkdisk <source_dir> <output.vhd> --size-mib 64 --format fat
cargo run -Z bindeps -- mkdisk <source_dir> <output.vhd> --size-mib 64 --format ext4
```

The default data size is 64 MiB. FAT accepts at least 4 MiB and automatically
selects FAT12/16/32; it does not force FAT32. ext4 accepts at least 16 MiB and
requires host `mkfs.ext4` and `debugfs` (e2fsprogs). FAT skips file symlinks;
ext4 preserves symlinks and copies hard links as independent files.

These are fixed version-1 VHDs: the footer stores unsigned 64-bit
original/current byte sizes, and the complete file must also fit the host's
signed 64-bit file-offset range. The 512-byte footer is additional to the
requested data size. CHS geometry is a legacy, rounded/clamped description,
not a capacity limit. No dynamic-VHD or VHDX capacity limit is imposed on
this fixed-image layout. FAT additionally has a 32-bit sector count with
512-byte sectors, allowing at most 2097151 whole MiB.
Size conversion, footer addition, signed 64-bit host file-offset limits, and
the format limit are checked before creating parent directories or opening
the destination. Invalid sizes leave existing destinations unchanged.
Valid requests still overwrite the destination; this is not atomic replacement
if formatting or copying fails.

### Root selection and `/etc/fstab`

`mkdisk` now installs `/etc/fstab` and its mountpoint directories into FAT and
ext4 root images, without modifying the source directory. It preserves the
contents of an existing source `etc/fstab`, or accepts an explicit override:

```bash
cargo run -Z bindeps -- mkdisk target/rootfs target/root.vhd --format ext4 --fstab ./my-fstab
cargo run -Z bindeps -- run uefi --vhd target/root.vhd
```

Without either input, the generated fstab is:

```fstab
/dev/sda /     ext4 defaults 0 0
proc     /proc proc defaults 0 0
dev      /dev  dev  defaults 0 0
```

FAT images use `fat` instead of `ext4`. Filesystem names here are Krust names;
`dev` is its devfs. The root layout must contain `bin/busybox`, not an extra
rootfs directory. The existing BusyBox setup script uses this image-creation
path automatically. FAT has no symlinks: its `sh` and `ash` entrypoints are
executable BusyBox copies, while other enabled applets use small scripts with
`#!/bin/busybox sh`. Only compiled applets are installed; no fake `bash` alias
is generated. Extra copies of the shell binary are the FAT space tradeoff.
Ext4 continues to use symlinks.

Existing FAT images must be regenerated to replace their recursive `/bin/sh`
wrapper. Until then, use `/bin/busybox sh` directly. Reinstall applets without
rebuilding BusyBox, then create a fresh image:

```bash
scripts/install-busybox-applets.sh target/busybox-build/busybox target/rootfs fat
cargo run -Z bindeps -- mkdisk target/rootfs target/root-fat.vhd --format fat
```

The kernel permits up to four interpreter redirects, then rejects the exec with
`ELOOP` rather than looping forever. This also protects against old recursive
scripts. Errors when launching from the kernel shell are logged with their
errno and the child exits with status 127; the parent shell remains responsive.
Userspace fork now resumes the child from its saved syscall register frame
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

Run the bounded FAT shell regression with an existing native BusyBox binary:

```bash
scripts/test-fat-shell.sh target/rootfs/bin/busybox
```

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

Both BIOS and UEFI run the complete matrix. A hung/crashed case fails after a
bounded deadline and the next case boots a fresh snapshot, so one broken
utility cannot prevent the remaining utilities from being attempted. The
source disk is hash-checked and temporary images/processes are cleaned up.
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

Each image also gets a `<image>.boot.toml` sidecar selecting its root filesystem
and device. The launcher reads this automatically and includes it in the boot
ramdisk for **all** console and BIOS/UEFI modes. Override it with:

```bash
cargo run -Z bindeps -- run bios --headless --vhd target/root.vhd --boot-config ./boot.toml
```

The flat boot settings include:

```toml
log_level = "info"
shell_port = 1
shell_console = "auto"
virtual_consoles = 6
syscall_trace = false
root_fs = "ext4"
root_device = "/dev/sda"
```

Only root selection must live in the boot ramdisk: the kernel cannot read a
disk's `/etc/fstab` until that root is mounted. A root fstab entry, if supplied,
must agree with the early settings; it is checked, not mounted twice.
Other fstab entries are validated before mounting, with parents before children.
Supported types are `ext4`, `fat`, `ramfs`, `proc`, and `dev`, subject to compiled
features. Disk sources must be explicit `/dev/...` paths; UUID/LABEL lookup and
automatic filesystem probing are not implemented.

Use six fstab fields, with final fields `0 0` (dump/fsck are not implemented).
Only `defaults` and `noauto` options are supported; unsupported options such as
`ro`, `rw`, `nofail`, or `bind` are rejected rather than silently ignored.
`defaults` uses the driver's current behavior, not the full Linux option set.
Blank lines, comments, and interior spaces escaped as `\040` are supported.
Targets must be canonical absolute paths without `.`/`..`, trailing slashes, or
duplicates. Boot configuration and fstab are each limited to 16 KiB; fstab has
at most 64 entries. Required mount failures stop boot with an explicit diagnostic.

Without a VHD, the launcher selects RAM root automatically; explicit RAM root
settings are `root_fs = "ramfs"` and `root_device = "none"`. The boot ramdisk
contains a matching fallback fstab, installed as `/etc/fstab` on RAM root.
For compatibility, old images without sidecars use ext4 `/dev/sda`; old disk
roots without `/etc/fstab` use the ramdisk fallback with a warning. An explicitly
requested missing disk root never silently falls back to RAM. Move/copy the
sidecar along with newly generated images.

Validate configuration, images, and real QEMU mounts with:

```bash
cargo test -Z bindeps --locked -p krust --bin krust
cargo build -Z bindeps --locked
scripts/test-boot-mounts.sh              # requires target/rootfs/bin/busybox and e2fsprogs
scripts/smoke-shell.sh target/krust-disk-ext4.vhd
```

The mount suite exercises FAT/ext4 roots, source/custom fstab preservation,
nested mount ordering, `noauto`, and BIOS/UEFI rejection of missing roots,
malformed/duplicate entries, root mismatches, unsupported options, and missing
required devices. Guest tests use snapshots and bounded process cleanup.
It does not certify ext4 mutations: [the mkdir EIO issue remains open (#51)](https://github.com/duggaraju/krust/issues/51).

### Running QEMU

Run UEFI with serial logs only:

```bash
cargo run -Z bindeps -- run uefi --vhd ./target/krust-disk-fat.vhd -- --display none --no-reboot --no-shutdown
```

If the VHD is locked, stop the previous QEMU process first:

```bash
ps -eo pid,cmd | grep qemu-system-x86_64 | grep -v grep
kill <PID>
```

To quickly repro BusyBox launch from stdin:

```bash
(sleep 8; printf '/bin/busybox ls\n'; sleep 3) | cargo run -Z bindeps -- run uefi --vhd ./target/krust-disk-fat.vhd -- --display none --no-reboot --no-shutdown
```

### PCI enumeration

The legacy PCI bus scans all bus/device numbers, reads function zero once, and
scans functions 1-7 only for multifunction devices. Every present function is
retained in the existing bus registry with vendor/device IDs, class/subclass/
programming interface, BDF, and header layout. Non-AHCI functions receive stable
`pciBB:DD.F` names and no device-type hint or BAR5. Only type-0 AHCI controllers
(`01:06:01`) retain the existing `sd*` names, block hint, masked BAR5, and SATA
binding; other PCI functions are enumeration-only, including virtio devices.
No BAR sizing, MSI, virtio driver, ECAM, hotplug, or resource assignment is provided.

Boot logs show at most 32 present functions (including non-AHCI vendor/device IDs)
and a total/omitted count; absent slots produce no logs. Run synthetic config
header tests without QEMU or kernel I/O:

```bash
scripts/test-pci.sh
```

To inspect an additional modern virtio RNG function without binding a driver
(use a directory under `target/` for the runner's generated files and trace):

```bash
mkdir -p target/pci-probe
TMPDIR="$PWD/target/pci-probe" timeout --kill-after=2s 30s \
  target/debug/krust run bios --headless --vhd target/krust-disk-ext4.vhd -- \
  -snapshot -no-reboot -device virtio-rng-pci,disable-legacy=on
```

The trace should contain `vendor=1af4 device=1044` with `ahci=false`, alongside
the existing AHCI controller. The timeout bounds this inspection and `-snapshot`
keeps the source VHD unchanged.

### BIOS/UEFI kernel-shell command regression

```bash
cargo build -Z bindeps --locked
scripts/test-smoke-userspace.sh
scripts/smoke-shell.sh target/krust-disk-ext4.vhd
```

This is a real QEMU **integration** suite for the kernel's built-ins, not
BusyBox replacements or host-only unit tests. It reuses the existing serial
prompt, deadline, isolated-process cleanup, and source-disk hash checks.
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

The host-only harness tests include 11 shell scenarios (success plus injected
state/content/diagnostic/clear/uptime/shutdown failures and timeout). Together
with the existing scenarios, all 33 check bounded execution, scratch removal,
owned child cleanup, and survival of an unrelated process. No QEMU or real
VHD is needed for those controlled tests; the four real boots validate the kernel.

### BIOS/UEFI userspace regression

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
`--headless` and `--pty` still select shell port 2; BIOS with only raw
`-display none`/`--display none` selects port 1. Ordinary boot console settings
are unchanged. Rebuild without `KRUST_BOOT_SYSCALL_TRACE` (or set it to `false`)
to disable these records; rerunning the smoke overwrites its logs.

### Descriptor close-on-exec (#20)

```bash
scripts/test-cloexec.sh
scripts/test-smoke-userspace.sh
cargo check -p kernel --target x86_64-unknown-none --locked
cargo build -Z bindeps --locked
scripts/test-cloexec-guest.sh target/krust-disk-ext4.vhd
scripts/smoke-userspace.sh target/krust-disk-ext4.vhd
```

`FD_CLOEXEC` is descriptor-local: `F_GETFD`/`F_SETFD` validate the descriptor
(`EBADF` even when setting zero), and only bit 0 is stored. `open`/`openat`
translate `O_CLOEXEC` separately from the access mode returned by `F_GETFL`.
`F_DUPFD` clears the new descriptor's flags; `F_DUPFD_CLOEXEC` sets them,
without changing the source. Allocation uses the lowest available descriptor,
including closed standard descriptors. Closing/reusing a slot clears its old
flags. Fork copies the flags independently; aliases retain a shared open file
description and offset, closed only when its last reference is released.
Read/write/seek no longer remove and reconstruct the descriptor's metadata.
Exec removes flagged entries **after** loading all segments, building the user
stack, committing and activating the new image. Failed exec leaves them intact.
The x86_64 non-returning exec entry also normalizes GS bases for both kernel
`fork_execve` and userspace `execve` callers, so the new image's first syscall
can use the kernel syscall stack.

Host tests compile the actual generic descriptor table used by tasks, not a
separate flag model. The guest harness compiles **two** static no-libc syscall
images, injects them and a fixed data file into an owned ext4 VHD **copy**, and
runs BIOS and UEFI with `-snapshot`. Both source and copied VHD hashes are
checked. It requires separate exact-line markers from the first image (failed
exec preserved flags), second image (successful exec removed them), and kernel
parent (fork inherited flags, child changes stayed local, shared offset survived).
It also exercises both open entry points, both fcntl duplication variants,
invalid descriptors, flag clearing, I/O retention, close/reuse and fd 0 reuse.
The complete ordinary BusyBox smoke follows in each boot; logs are retained in
`target/cloexec-guest-{bios,uefi}{,-kernel}.log`. Owned binaries, disk copies,
scratch directories and processes are cleaned up. Requires existing `cc`,
`debugfs`, and the ordinary smoke prerequisites; no dependency changes.

**Bounded scope:** userspace resumable fork remains blocked (#48); the
debug-only kernel shell command `cloexec-test` uses the existing `fork_execve`
path and checks the parent's surviving descriptors. The guest harness therefore
requires a debug build. `execve` still ignores userspace argv/envp (#49); the two
images use separate fixed executable paths and a fixed fd protocol instead.
`dup`, `dup2`, `dup3`, `pipe`, and `pipe2` are not currently dispatched and were
not added. Existing openat dirfd/path limitations and incomplete file-status
flags (`F_SETFL`, append/nonblocking, etc.) are unchanged.

### mmap range and protection regressions (#9, #8)

```bash
scripts/test-mmap.sh
scripts/test-mmap-protection-harness.sh
cargo check -p kernel --target x86_64-unknown-none --locked
cargo build -Z bindeps --locked
scripts/test-mmap-guest.sh target/krust-disk-ext4.vhd
scripts/smoke-userspace.sh target/krust-disk-ext4.vhd
```

The host tests execute the same pure range helper used by `sys_mmap`, using
the actual x86_64 layout constants, and directly test the read-only page-table
walker with synthetic tables. They also cross-compile the actual ARM64 layout
and paging-helper surface independently of that port's existing kernel build
failures. Before mapping any pages, anonymous mmap
checks page rounding, end addition, and the exclusive four-level lower-canonical
user limit (`2^47`). Zero length returns `EINVAL`; overflow, non-user addresses,
and ranges crossing that limit return `ENOMEM`. A range ending exactly at the
limit is valid. A read-only page-table preflight also rejects any overlap with
present supervisor-only mappings, including inherited lower-half kernel pages.
Effective user permissions include **every parent entry**, and both 2-MiB and
1-GiB mappings are checked. Unmapped subtrees are skipped without scanning every
page of a large range. The complete check runs before any page or mmap cursor
is changed; existing user-user overlap behavior is untouched.
Unsupported file mappings, anonymous descriptors other than
`-1`/`u32::MAX`, and `MAP_FIXED` still return `ENOMEM`, with their original
precedence over length validation.

The guest test compiles a static, no-libc x86_64 C syscall fixture with the
installed `cc`, injects it with `debugfs` into an owned **copy** of the ext4 VHD,
and runs the prompt-driven BIOS/UEFI smoke harness with QEMU `-snapshot`.
Invalid calls are interleaved with healthy zero-filled allocations; unchanged
placement and sentinel pages check that rejected ranges do not change existing
mappings or the default cursor. Valid tests cover unaligned hints/lengths and
the exact upper boundary. Each boot derives a real kernel page from the
bootloader's logged relocation bias plus first `PT_LOAD` virtual address,
and passes it to the guest (no assumed kernel load address). The guest rejects
both that address and a range from a live ELF data sentinel into the kernel,
then verifies its sentinel, existing allocations, and placement are intact.
Missing address provenance or either guest success marker fails the harness.
The boundary page is in the currently unused top of
the reserved initial stack, well above its live stack pointer; replacing that
page intentionally exercises the existing mapper, not a new collision policy.
The complete BusyBox smoke runs **after** the fixture in each boot.
Both the copied disk and source disk are hash-checked; owned fixture binaries,
disk copies, and scratch directories are removed. Logs remain in
`target/mmap-guest-{bios,uefi}{,-kernel}.log`. Requires `cc` and `debugfs`
in addition to the smoke prerequisites; no libc or new dependencies are needed.

**Existing limitations intentionally retained:** unlike
[Linux mmap hints](https://man7.org/linux/man-pages/man2/mmap.2.html), Krust
always tries a nonzero hint rounded **up**, with no alternative-gap search or
fallback placement. Consequently a hint whose rounded range is outside the
user limit is rejected, even where Linux could ignore it and allocate elsewhere.
In-range hints are not rejected merely for being unaligned. Zero hints retain
the existing monotonic placement beginning at the architecture's mmap base.
This change does not implement `mprotect` (#7) or overlap/replacement
policy between user mappings (#10). ARM64 now also owns the mmap layout constants;
its existing unimplemented page-table inspection fails closed rather than
claiming a range is safe.

Anonymous x86_64 `mmap` accepts all combinations of `PROT_READ`, `PROT_WRITE`,
and `PROT_EXEC`; unknown protection bits return `EINVAL` after the existing
unsupported-mode and zero-length checks. Non-executable mappings use NX;
read-only mappings omit write permission. Hardware permits reads on write-only
and execute-only pages too; no `READ_IMPLIES_EXEC` personality is implemented.
`PROT_NONE` retains its allocated frame, USER identity, and ownership record,
but uses a software-marked non-present leaf, denying reads, writes, and execution.
Parent tables remain user-accessible; kernel initialization writes through the
physical-memory alias, not the protected user address. Boot explicitly enables
EFER.NXE and CR0.WP. ARM64's actual protection-helper surface is cross-compiled,
but valid protections return `ENOSYS`: this is not ARM64 runtime support.

The guest script additionally boots BIOS and UEFI protection probes in separate
kernel-shell-launched processes, avoiding the known userspace `fork` bug (#48).
It exercises all eight combinations (including executing EXEC-only/RX zero-page
tails into an adjacent executable RET), then attempts nine forbidden writes,
instruction fetches, and `PROT_NONE` accesses. Each must produce its exact
user-mode page-fault class and task status 139; the shell must run a healthy
BusyBox child after every fault. Missing faults/statuses fail the focused harness.
Expected faults are allowed only there, never in the standard smoke harness.
Logs: `target/mmap-protections-{bios,uefi}{,-kernel}.log`. The host-only protection
harness tests reject missing/wrong faults, missing statuses, and unrelated faults.

Requirements: Bash, GNU `timeout`, `realpath`, `sha256sum`, util-linux `setsid`,
and the normal QEMU/build prerequisites.

Run the focused host-only harness tests without QEMU or a real VHD:

```bash
scripts/test-smoke-userspace.sh
```

The controlled fake runner checks split prompts and rejects premature/batched
commands. Tests cover success, missing shell, command failure, boot/mid-command
and closed-output timeouts, echoed markers without output, missing pwd/proc
content, expected syscall-probe traces versus unrelated kernel errors, runner
failure, invalid deadlines/paths/arguments, and disk mutation on failure and
success. They verify owned child processes are reaped, scratch directories
removed, and an unrelated process left alive. Each case uses a three-second
deadline; case logs remain in `target/userspace-smoke-test-*.log`.

This is an entry/IRQ regression, not certification of userspace `fork`.
An ash subprocess sequence also faults on the original, unmodified UEFI
kernel during userspace fork (a ring-0 instruction-fetch
protection fault). Separately, the existing `sys_execve` handler ignores
userspace argv/envp, so `env /bin/busybox echo ...` loses its arguments.
Those pre-existing syscall limitations are outside this boot-entry fix;
the probe deliberately uses the kernel shell's `fork_execve` path.

The kernel must establish CPU/device state instead of inheriting firmware
defaults. In particular, BIOS can leave the legacy PIC's IRQ0 at vector `0x08`
(the double-fault vector), while UEFI masks the PIC. A pending timer interrupt
then enters the double-fault IST before the first userspace instruction; an
immutable IST stack in read-only memory misleadingly produces a supervisor
write fault at the ELF entry. The kernel remaps IRQs to `0x20..0x2f`,
enables only the handled PIT IRQ, acknowledges interrupts, and allocates a
writable exception stack. BIOS also leaves CR4.OSFXSR clear, unlike UEFI,
causing BusyBox's first SSE instruction to fault. Explicit x87/SSE initialization
fixes that second firmware dependency. Kernel page permissions and CR0.WP
remain unchanged.

### Runner exit codes

The `run` command enables QEMU's `isa-debug-exit` device. A guest write is
encoded as `(guest_value << 1) | 1` in QEMU's exit status:

| Guest value | Raw QEMU status | Host runner status |
|-------------|-----------------|--------------------|
| `0x10` (success) | `33` | `0` (success) |
| `0x11` (failure) | `35` | `1` (failure) |

Only raw status `33` is success. Every other QEMU status (including `0`),
termination without an exit code (such as a signal), and QEMU launch or wait
errors produce a diagnostic and host status `1`.
