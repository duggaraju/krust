# Building and running Krust

[Project overview](../Readme.md) | [Testing](testing/README.md)

Run all commands from the repository root.

## Toolchain and prerequisites

Install the repository toolchain:

```bash
rustup toolchain install
```

`rust-toolchain.toml` pins `nightly-2026-03-01`, including
`llvm-tools-preview`, `rust-src` (for building the bootloader), and the
`x86_64-unknown-none` and `aarch64-unknown-none` targets. Cargo automatically
uses this pin in the repository. Do not override it with `+nightly`: newer
floating nightlies can break the unstable `Step` trait implementations in the
locked `x86_64` dependencies. Keep dependency versions and `Cargo.lock`
unchanged when setting up the toolchain.

Install the BusyBox rootfs build tools:

```bash
sudo apt install build-essential wget libncurses-dev musl-tools e2fsprogs
```

To build BusyBox with glibc instead of musl, pass `--libc glibc` to
`scripts/setup-busybox.sh`. See [BusyBox and userspace](busybox.md) for rootfs
layout and filesystem-specific applet installation.

## Build and check

```bash
cargo build -Z bindeps --locked
cargo check -p kernel --target x86_64-unknown-none --locked
```

The kernel-only check avoids building disk images.

## Running QEMU

```bash
cargo run -Z bindeps -- run uefi
cargo run -Z bindeps -- run bios
```

For UEFI with serial logs only and an existing data disk:

```bash
cargo run -Z bindeps -- run uefi --vhd ./target/krust-disk-fat.vhd -- --display none --no-reboot --no-shutdown
```

See [storage and boot configuration](storage.md) to create a disk and configure
root selection or mounts.
Use `init=/bin/sh` before any raw-QEMU `--` arguments to choose the PID 1
executable. Without an override, `/sbin/init` is used when present; otherwise
the kernel shell runs as PID 1. `KRUST_BOOT_INIT` provides a build-time default.

If the VHD is locked, identify and stop the previous QEMU process:

```bash
ps -eo pid,cmd | grep qemu-system-x86_64 | grep -v grep
kill <PID>
```

To quickly reproduce BusyBox launch from stdin:

```bash
(sleep 8; printf '/bin/busybox ls\n'; sleep 3) | cargo run -Z bindeps -- run uefi --vhd ./target/krust-disk-fat.vhd -- --display none --no-reboot --no-shutdown
```

## Runner exit codes

The `run` command enables QEMU's `isa-debug-exit` device. A guest write is
encoded as `(guest_value << 1) | 1` in QEMU's exit status:

| Guest value | Raw QEMU status | Host runner status |
|-------------|-----------------|--------------------|
| `0x10` (success) | `33` | `0` (success) |
| `0x11` (failure) | `35` | `1` (failure) |

Only raw status `33` is success. Every other QEMU status (including `0`),
termination without an exit code (such as a signal), and QEMU launch or wait
errors produce a diagnostic and host status `1`.
