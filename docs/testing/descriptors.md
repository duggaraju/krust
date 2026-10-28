# Descriptor close-on-exec

[Testing index](README.md) | [Current process behavior](../busybox.md#process-and-interpreter-behavior)

Run all commands from the repository root. This documents the #20 regression.

```bash
cargo test -Z bindeps --locked -p krust --test cloexec
cargo test -Z bindeps --locked -p krust --test harnesses
cargo test -Z bindeps --locked -p krust --test qemu cloexec -- --ignored --exact --nocapture
cargo test -Z bindeps --locked -p krust --test qemu userspace_smoke -- --ignored --exact --nocapture
```

The native `cloexec` target runs host tests; the ignored `qemu::cloexec` case
boots both firmware modes. See [test selection](README.md#cargo-targets-and-filtering)
for fixture overrides and additional filters.

## Semantics

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

## Coverage and requirements

Host tests compile the actual generic descriptor table used by tasks, not a
separate flag model. Cargo builds **two** static `no_std` Rust syscall
images from [the guest package](../../tests/guest/Cargo.toml).
The guest harness injects them and a fixed data file into an owned ext4 VHD **copy**, and
runs BIOS and UEFI with `-snapshot`. Both source and copied VHD hashes are
checked. It requires separate exact-line markers from the first image (failed
exec preserved flags), second image (successful exec removed them), and kernel
parent (fork inherited flags, child changes stayed local, shared offset survived).
It also exercises both open entry points, both fcntl duplication variants,
invalid descriptors, flag clearing, I/O retention, close/reuse and fd 0 reuse.

The complete ordinary BusyBox smoke follows in each boot; logs are retained in
the unique `target/cargo-tests/native/cloexec-*/` directory printed by the harness.
Owned binaries, disk copies,
scratch directories and processes are cleaned up. Requires `debugfs`, the pinned
Rust toolchain, and the ordinary smoke prerequisites; no C compiler or libc.

## Bounded scope

The debug-only kernel shell command `cloexec-test` uses the kernel `fork_execve`
path and checks the parent's surviving descriptors, so this guest harness
requires a debug build. Its two images use separate fixed executable paths and
a fixed fd protocol: the harness was introduced before userspace fork and
argv/envp fixes (#48/#49), which now have their own
[process regression](shell.md#busybox-shell-and-process-regression).

`dup`, `dup2`, `dup3`, `pipe`, and `pipe2` are not currently dispatched and were
not added by the CLOEXEC work. Existing openat dirfd/path limitations and
incomplete file-status flags (`F_SETFL`, append/nonblocking, etc.) are unchanged.
