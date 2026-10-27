# Krust

A bare-metal x86_64 monolithic kernel written in Rust, working toward Linux
syscall compatibility. The workspace contains the kernel and a host tool for
building disk images and running QEMU.

## Quick start

Run from the repository root with QEMU installed:

```bash
rustup toolchain install
cargo build -Z bindeps --locked
cargo run -Z bindeps -- run uefi
# Or: cargo run -Z bindeps -- run bios
```

The repository pins its nightly toolchain; do not replace it with a floating
`+nightly` or update dependencies just to build. See the
[development guide](docs/development.md) for prerequisites and troubleshooting.

## Testing

```bash
# Host tests; no QEMU boots.
cargo test -Z bindeps --locked -p krust

# Select one guest utility case; requires an existing BusyBox binary.
cargo test -Z bindeps --locked -p krust --test busybox fat::bios::ls -- --ignored --exact
```

QEMU tests are opt-in. Passing tests do not imply complete Linux compatibility;
unsupported behavior remains an explicit failure. See the
[testing guide](docs/testing/README.md) for targets, filters, fixtures and logs.

## Documentation

| Guide | Contents |
|-------|----------|
| [Development](docs/development.md) | Toolchain, build/check commands, QEMU workflow and exit codes |
| [Storage and boot configuration](docs/storage.md) | FAT/ext4 images, root selection, boot sidecars and `/etc/fstab` |
| [BusyBox and userspace](docs/busybox.md) | Applet installation, process semantics and utility compatibility |
| [Testing](docs/testing/README.md) | Cargo targets, case selection, prerequisites and report locations |
| [Shell and userspace regressions](docs/testing/shell.md) | Built-ins, interactive shells, fork/exec/wait and syscall tracing |
| [Descriptor regressions](docs/testing/descriptors.md) | CLOEXEC semantics and host/guest coverage |
| [Memory regressions](docs/testing/memory.md) | mmap bounds, permissions, expected faults and architecture limitations |
| [PCI enumeration](docs/pci.md) | Supported discovery behavior, host tests and QEMU inspection |
