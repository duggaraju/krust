# Copilot Instructions

## Architecture

This is a bare-metal x86_64 monolithic kernel written in Rust, aiming for Linux syscall compatibility. The workspace has two crates:

- **`kernel/`** — The `#![no_std]` kernel binary, compiled for `x86_64-unknown-none`. Uses `bootloader_api` with the `entry_point!` macro. Subsystems are gated behind cargo features.
- **Root crate (`src/main.rs`)** — A QEMU runner/tool host built with `clap`. Subcommands: `run <uefi|bios>` and `mkdisk <dir> <out.vhd>`. `build.rs` creates UEFI/BIOS disk images and a cpio ramdisk containing `boot.toml`.

### Kernel Subsystems (`kernel/src/`)

| Directory | Feature Flag | Purpose |
|-----------|-------------|---------|
| `arch/` | always | Architecture abstraction layer (portable traits + x86_64 impl) |
| `syscall/` | always | Linux-compatible syscall dispatch (always compiled) |
| `mm/` | `mm` | Memory management (frame allocator, heap, paging) |
| `process/` | `process` (requires `mm`) | Tasks, scheduler, context switching, binfmt |
| `fs/` | `fs` | VFS traits, ramfs, initrd, FAT32, devfs, procfs |
| `drivers/` | `drivers` | Device driver framework (char/block traits, serial, video, tty) |
| `module/` | `modules` (requires `fs`) | Loadable kernel module support |
| `shell/` | `shell` | Built-in interactive shell |

`fs` and `drivers` both have granular sub-features (e.g. `fs-ramfs`, `fs-fat32`, `fs-proc`, `device-serial`, `device-video`, `device-tty`). The umbrella features (`fs`, `drivers`) enable all sub-features. The default feature set enables everything.

### VFS & Filesystems

- `fs/vfs.rs` defines the core traits: `Inode`, `FileSystem`, `DirEntry` (borrowed, streaming), `FsError`, `FileOpenMode`, `SeekFrom`.
- `fs/mod.rs` defines `FileSystemFactory` — the trait modules implement to register mountable filesystems. Block-backed filesystems (FAT32) take an `Arc<dyn BlockDevice>`; virtual filesystems (procfs, devfs) do not.
- Mounts are registered with `fs::register_mount(path, fs_name, device)` and resolved at `fs::mount_registered_filesystems()`. Mount specs are stored by name; the filesystem object is looked up at mount time.
- `readdir` synthesizes `.` and `..` entries before delegating to the filesystem.

### Module System

Modules implement the `KernelModule` trait (`init`, `cleanup`, `dependencies`). `cleanup` receives a `&dyn KernelRegistry` so modules can unregister resources (filesystems, devices, binfmt handlers, buses). `KernelRegistry::unregister_filesystem_factory` returns `Busy` if any active mounts use the filesystem. Modules live alongside their owning subsystem (e.g. `fs/procfs.rs`, `fs/devfs.rs`) rather than in `module/`.

### Boot Configuration

Boot settings are passed via a cpio ramdisk containing `boot.toml`. Two env vars control the generated config at build time:

- `KRUST_BOOT_LOG_LEVEL` — kernel log level (default: `info`)
- `KRUST_BOOT_SHELL_CONSOLE` — `auto` | `framebuffer` | `serial` (default: `auto`)

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

# Run in QEMU (note: subcommand is 'run')
cargo run -Z bindeps -- run uefi
cargo run -Z bindeps -- run bios

# Extra options for the run subcommand
cargo run -Z bindeps -- run uefi --headless       # no display window
cargo run -Z bindeps -- run uefi --pty            # attach serial to PTY
cargo run -Z bindeps -- run uefi --vhd <path>     # attach a VHD disk

# Create a FAT32 VHD from a directory (e.g. for /bin contents)
cargo run -Z bindeps -- mkdisk <source_dir> <out.vhd>
scripts/setup-busybox.sh                          # populate a busybox sysroot

# Check kernel only (faster iteration)
cd kernel && cargo check --target x86_64-unknown-none
cd kernel && cargo check --target x86_64-unknown-none --all-features

# Check a specific feature subset
cargo check -p kernel --target x86_64-unknown-none --features syscall
```

## Key Conventions

- `panic = "abort"` in all profiles — no unwinding.
- `#![feature(abi_x86_interrupt)]` enabled for IDT handlers.
- Use `spin::Mutex` / `spin::Lazy` for static synchronization (no std Mutex).
- `linked_list_allocator::LockedHeap` is the `#[global_allocator]`.
- Logging via the `log` crate; framebuffer logger from `bootloader_x86_64_common`. Kernel traces go to COM1 (0x3F8) when framebuffer UI is active.
- Subsystems expose `pub fn init()` called from `kernel_main` in feature-gated order.
- Use `extern crate alloc;` in subsystem modules that need heap allocation.
- VGA output through the `vga` crate, serial through `uart_16550`.
- Rust 2024 edition for both crates.
- QEMU exit codes: `0x10` = success, `0x11` = failure.
- Shell file/directory commands resolve relative paths from the current process cwd inode via syscall-layer helpers, not direct VFS lookups.

