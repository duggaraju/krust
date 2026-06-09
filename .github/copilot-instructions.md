# Copilot Instructions

## Architecture

This is a bare-metal x86_64 monolithic kernel written in Rust, aiming for Linux syscall compatibility. The workspace has two crates:

- **`kernel/`** — The `#![no_std]` kernel binary, compiled for `x86_64-unknown-none`. Uses `bootloader_api` with the `entry_point!` macro. Subsystems are gated behind cargo features.
- **Root crate (`src/main.rs`)** — A QEMU runner that builds UEFI/BIOS disk images via `build.rs` and launches QEMU.

### Kernel Subsystems (`kernel/src/`)

| Directory | Feature Flag | Purpose |
|-----------|-------------|---------|
| `arch/` | always | Architecture abstraction layer (portable traits + x86_64 impl) |
| `mm/` | `mm` | Memory management (frame allocator, heap, paging) |
| `process/` | `process` | Process management (tasks, scheduler, context switching) |
| `fs/` | `fs` | Virtual filesystem (VFS traits, ramfs) |
| `syscall/` | `syscall` | Linux-compatible syscall dispatch |
| `drivers/` | `drivers` | Device driver framework (char/block traits, serial) |
| `module/` | `modules` | Loadable kernel module support |

Feature dependencies: `process` requires `mm`, `syscall` requires `process` + `fs`.

### Portability

- All arch-specific code lives under `kernel/src/arch/{arch_name}/`
- Portable traits defined in `kernel/src/arch/mod.rs`
- Use `#[cfg(target_arch = "...")]` for arch-conditional code
- Address types in `mm/address.rs` provide a portability shim

## Build & Run

Requires Rust nightly (pinned in `rust-toolchain.toml`) and `llvm-tools-preview`.

```bash
# Build everything
cargo build -Z bindeps

# Run in QEMU
cargo run -Z bindeps -- uefi
cargo run -Z bindeps -- bios

# Check kernel only (faster iteration)
cd kernel && cargo check --target x86_64-unknown-none
cd kernel && cargo check --target x86_64-unknown-none --all-features
```

## Key Conventions

- `panic = "abort"` in all profiles — no unwinding.
- `#![feature(abi_x86_interrupt)]` enabled for IDT handlers.
- Use `spin::Mutex` / `spin::Lazy` for static synchronization (no std Mutex).
- `linked_list_allocator::LockedHeap` is the `#[global_allocator]`.
- Logging via the `log` crate; framebuffer logger from `bootloader_x86_64_common`.
- Subsystems expose `pub fn init()` called from `kernel_main` in feature-gated order.
- Use `extern crate alloc;` in subsystem modules that need heap allocation.
- VGA output through the `vga` crate, serial through `uart_16550`.
- Rust 2024 edition for both crates.
- QEMU exit codes: `0x10` = success, `0x11` = failure.

