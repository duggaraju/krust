# mmap range and protection regressions

[Testing index](README.md) | [Userspace smoke](shell.md#biosuefi-userspace-smoke)

Run all commands from the repository root. These cover the #9 and #8 regressions.

```bash
scripts/test-mmap.sh
scripts/test-mmap-protection-harness.sh
cargo check -p kernel --target x86_64-unknown-none --locked
cargo build -Z bindeps --locked
scripts/test-mmap-guest.sh target/krust-disk-ext4.vhd
scripts/smoke-userspace.sh target/krust-disk-ext4.vhd
```

Cargo exposes the native `mmap` target and the ignored `qemu` target's `mmap`
case; see [test selection](README.md#cargo-targets-and-filtering).
`scripts/test-mmap.sh` additionally performs the cross-target compile check.

## Range validation and supervisor overlap

The host tests execute the same pure range helper used by `sys_mmap`, using
the actual x86_64 layout constants, and directly test the read-only page-table
walker with synthetic tables. They also cross-compile the actual ARM64 layout
and paging-helper surface independently of that port's existing kernel build
failures.

Before mapping any pages, anonymous mmap checks page rounding, end addition,
and the exclusive four-level lower-canonical user limit (`2^47`). Zero length
returns `EINVAL`; overflow, non-user addresses, and ranges crossing that limit
return `ENOMEM`. A range ending exactly at the limit is valid. A read-only
page-table preflight also rejects any overlap with present supervisor-only
mappings, including inherited lower-half kernel pages. Effective user
permissions include **every parent entry**, and both 2-MiB and 1-GiB mappings
are checked. Unmapped subtrees are skipped without scanning every page of a
large range. The complete check runs before any page or mmap cursor is changed;
existing user-user overlap behavior is untouched.

Unsupported file mappings, anonymous descriptors other than `-1`/`u32::MAX`,
and `MAP_FIXED` still return `ENOMEM`, with their original precedence over
length validation.

## Range guest fixture

The guest test compiles a static, no-libc x86_64 C syscall fixture with the
installed `cc`, injects it with `debugfs` into an owned **copy** of the ext4 VHD,
and runs the prompt-driven BIOS/UEFI smoke harness with QEMU `-snapshot`.
Invalid calls are interleaved with healthy zero-filled allocations; unchanged
placement and sentinel pages check that rejected ranges do not change existing
mappings or the default cursor. Valid tests cover unaligned hints/lengths and
the exact upper boundary.

Each boot derives a real kernel page from the bootloader's logged relocation
bias plus first `PT_LOAD` virtual address, and passes it to the guest (no assumed
kernel load address). The guest rejects both that address and a range from a
live ELF data sentinel into the kernel, then verifies its sentinel, existing
allocations, and placement are intact. Missing address provenance or either
guest success marker fails the harness. The boundary page is in the currently
unused top of the reserved initial stack, well above its live stack pointer;
replacing that page intentionally exercises the existing mapper, not a new
collision policy.

The complete BusyBox smoke runs **after** the fixture in each boot.
Both the copied disk and source disk are hash-checked; owned fixture binaries,
disk copies, and scratch directories are removed. Logs remain in
`target/mmap-guest-{bios,uefi}{,-kernel}.log`. Requires `cc` and `debugfs`
in addition to the smoke prerequisites; no libc or new dependencies are needed.

## Existing placement and architecture limitations

Unlike [Linux mmap hints](https://man7.org/linux/man-pages/man2/mmap.2.html),
Krust always tries a nonzero hint rounded **up**, with no alternative-gap search
or fallback placement. Consequently a hint whose rounded range is outside the
user limit is rejected, even where Linux could ignore it and allocate elsewhere.
In-range hints are not rejected merely for being unaligned. Zero hints retain
the existing monotonic placement beginning at the architecture's mmap base.

This does not implement `mprotect` (#7) or overlap/replacement policy between
user mappings (#10). ARM64 owns the mmap layout constants; its existing
unimplemented page-table inspection fails closed rather than claiming a range
is safe.

## Protection semantics

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

## Protection guest and harness coverage

The guest script boots BIOS and UEFI protection probes in separate
kernel-shell-launched processes, independently of the userspace fork regression.
It exercises all eight combinations (including executing EXEC-only/RX zero-page
tails into an adjacent executable RET), then attempts nine forbidden writes,
instruction fetches, and `PROT_NONE` accesses. Each must produce its exact
user-mode page-fault class and task status 139; the shell must run a healthy
BusyBox child after every fault. Missing faults/statuses fail the focused harness.
Expected faults are allowed only there, never in the standard smoke harness.

Logs are in `target/mmap-protections-{bios,uefi}{,-kernel}.log`. The host-only
protection harness tests reject missing/wrong faults, missing statuses, and
unrelated faults. See the [shell harness guide](shell.md#host-only-harness-checks)
for shared tool requirements and mock-runner cleanup assertions.
