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
Supported types are `ext4`, `fat`, `ramfs`, `proc`, and `dev`, subject to compiled
features. Disk sources must be explicit `/dev/...` paths; UUID/LABEL lookup and
automatic filesystem probing are not implemented.

## Fstab validation and fallback behavior

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

## Mount regression tests

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
It does not certify ext4 mutations:
[the mkdir EIO issue remains open (#51)](https://github.com/duggaraju/krust/issues/51).

The suite is also available as the `boot_mounts` Cargo integration test;
see [Cargo test targets](testing/README.md).
