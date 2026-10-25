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
(sleep 8; printf '/bin/bin/busybox ls\n'; sleep 3) | cargo run -Z bindeps -- run uefi --vhd ./target/krust-disk-fat.vhd -- --display none --no-reboot --no-shutdown
```