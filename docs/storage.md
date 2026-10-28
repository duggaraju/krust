# Storage and boot configuration

[Project overview](../Readme.md) | [Development](development.md) |
[BusyBox](busybox.md) | [Testing](testing/README.md)

Run all commands from the repository root.

## Creating data disks

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

## Root selection and `/etc/fstab`

`mkdisk` installs `/etc/fstab` and its mountpoint directories into FAT and
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
rootfs directory. The BusyBox setup script uses this image-creation path
automatically. See [applet installation](busybox.md#fat-and-ext4-applet-installation)
for FAT wrappers versus ext4 symlinks.

## Boot sidecars and early settings

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
Supported types are `ext4`, `fat`, `ramfs`, `tmpfs`, `proc`, and `dev`, subject to compiled
features. Disk sources must be explicit `/dev/...` paths; UUID/LABEL lookup and
automatic filesystem probing are not implemented.

## Fstab validation and fallback behavior

Use six fstab fields, with final fields `0 0` (dump/fsck are not implemented).
Except for the tmpfs-specific options below, only `defaults` and `noauto` options
are supported; unsupported options such as
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

## PID 1 and `init=`

After mounting root and the configured filesystems, the kernel starts PID 1:

1. If `init` is specified, execute that path.
2. Otherwise, execute `/sbin/init` if it exists.
3. If `/sbin/init` is absent, run the built-in kernel shell itself as PID 1.

There is no placeholder PID 1 supervisor or separate PID 2 kernel shell.
PID 1 has parent PID 0. Userspace init starts in `/` with root credentials,
console-backed stdin/stdout/stderr, and `HOME=/`, `TERM=linux`, and
`PATH=/sbin:/bin:/usr/sbin:/usr/bin`. Its initial argv contains its executable
path; ELF and shebang loading use the normal exec path.

Supply a Linux-style parameter to the runner, **before** the `--` introducing
raw QEMU arguments:

```bash
cargo run -Z bindeps -- run uefi --headless --vhd target/root.vhd init=/bin/sh
```

Alternatively, add `init = "/bin/sh"` to boot TOML, or set `KRUST_BOOT_INIT`
at build time. The runner parameter overrides the boot TOML value, which
overrides the build-time default. Omit the setting for automatic selection;
empty, relative, or noncanonical paths are rejected. Only the executable path,
not an init command line or arguments, is supported.

An explicitly selected init never silently falls back. An existing but
unexecutable `/sbin/init` also fails boot, including broken symlinks. Missing
or invalid executables report their path and errno. Userspace PID 1 exiting
or suffering a fatal fault/signal stops the kernel with failure; the fallback
kernel shell retains its normal successful `exit`/`shutdown` behavior.
If the kernel shell is disabled and no init exists, boot fails.

This selects and launches init; it does not add Linux orphan adoption or a
service manager. Userspace init must reap its children and implement its own
startup policy.

## RAM filesystems

`ramfs` and `tmpfs` share the in-memory inode/directory implementation, including
metadata, rename, truncation, removal, and open-unlinked file lifetimes. They
use different storage policies: ramfs retains contiguous heap-backed file data
with no filesystem quota; tmpfs uses sparse, zero-filled 4 KiB physical pages
outside the kernel heap, with independent limits for each mount.

Enable `fs-tmpfs` (included in the default `fs` umbrella) and add entries to
the root image's `/etc/fstab`, for example:

```fstab
tmpfs /tmp tmpfs size=16M,nr_inodes=1024,mode=1777 0 0
none  /run tmpfs size=4M,nr_inodes=256,mode=0755,uid=0,gid=0 0 0
```

Tmpfs is currently a non-root mount; early root selection remains
`ext4`, `fat`, or `ramfs`. No tmpfs mounts are installed automatically.

| Option | Default | Meaning |
|--------|---------|---------|
| `size` | `16M` | Positive byte limit, rounded up to 4 KiB; optional `K`, `M`, `G` suffix (case-insensitive) |
| `nr_inodes` | `1024` | Positive inode limit, including the root directory |
| `mode` | `0755` | Root directory permission bits, in octal |
| `uid`, `gid` | `0`, `0` | Root ownership and initial ownership of directly created VFS inodes |

Limits do not reserve memory upfront. Only allocated file pages consume the
byte quota: seeking or extending with truncate leaves holes without allocating
pages. Inode and page-index metadata still use the kernel heap; the inode limit
also bounds directory entries (names are limited to 255 bytes). Quota exhaustion
returns `ENOSPC`; tmpfs data/metadata allocation failures return `ENOMEM`.
Writes may return a short count if only part fits. Existing pages remain writable
at the limit. Truncation releases pages and zeroes the retained page's tail.
Unlink removes the name immediately, but pages and the inode remain charged
until the last strong reference, including open descriptors, is released.

The in-memory backends support create/exclusive-open, truncate, atomic append,
mkdir/rmdir, unlink, rename within one mount, and uid/gid/mode updates. Tmpfs
`statfs`/`fstatfs` report page/inode capacity and free counts; file `stat` reports
allocated 512-byte blocks rather than charging sparse holes. The new `*at`
operations support absolute paths and `AT_FDCWD`; other relative dirfds fail
explicitly. Other filesystem drivers retain unsupported-operation errors for
VFS methods they do not implement.

This is not full Linux tmpfs compatibility: the kernel does not yet provide
complete permission/sticky-bit enforcement, umask, timestamps, or hard-link
syscalls. Storing `mode=1777` does not itself enforce sticky-directory rules.
There is no swap or file-backed mmap; `/dev/shm` alone would therefore not supply
shared-memory mappings. Follow-ups are [shared mappings (#68)](https://github.com/duggaraju/krust/issues/68)
and [swap/reclaim (#69)](https://github.com/duggaraju/krust/issues/69).

### RAM filesystem regression tests

```bash
cargo test -Z bindeps --locked -p krust --test memfs --bin krust
cargo test -Z bindeps --locked -p krust --test qemu tmpfs -- --ignored --exact --nocapture
```

Host tests cover sparse holes, exact byte/inode limits, partial writes,
allocation-failure rollback, truncation, metadata, atomic append, concurrent
reservations, cross-directory rename/cycle rejection, mount independence,
ramfs behavior, and final-close reclamation through VFS.
The guest suite uses a Cargo-built `no_std` Rust probe and builds an owned ext4
fixture using an existing BusyBox, tests
real syscalls on BIOS and UEFI, verifies nested mounts, writes 3 MiB (larger than
the kernel heap), and repeatedly reuses freed frames. It does not modify the
source BusyBox or existing disks. Logs are retained as
the unique `target/cargo-tests/native/tmpfs-*/` directory printed by the Rust harness.

## Mount regression tests

```bash
cargo test -Z bindeps --locked -p krust --bin krust
# Requires target/rootfs/bin/busybox and e2fsprogs.
cargo test -Z bindeps --locked -p krust --test qemu boot_mounts -- --ignored --exact --nocapture
cargo test -Z bindeps --locked -p krust --test qemu shell_commands -- --ignored --exact --nocapture
```

The mount suite exercises FAT/ext4 roots, source/custom fstab preservation,
nested mount ordering, `noauto`, and BIOS/UEFI rejection of missing roots,
malformed/duplicate entries, root mismatches, unsupported options, and missing
required devices. Guest tests use snapshots and bounded process cleanup.
It does not certify ext4 mutations:
[the mkdir EIO issue remains open (#51)](https://github.com/duggaraju/krust/issues/51).

See [Cargo test targets](testing/README.md) for fixture overrides and test filtering.
