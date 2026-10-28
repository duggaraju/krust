# PCI enumeration

[Project overview](../Readme.md) | [Development](development.md) |
[Testing](testing/README.md)

Run all commands from the repository root.

The legacy PCI bus scans all bus/device numbers, reads function zero once, and
scans functions 1-7 only for multifunction devices. Every present function is
retained in the existing bus registry with vendor/device IDs, class/subclass/
programming interface, BDF, and header layout. Non-AHCI functions receive stable
`pciBB:DD.F` names and no device-type hint or BAR5. Only type-0 AHCI controllers
(`01:06:01`) retain the existing `sd*` names, block hint, masked BAR5, and SATA
binding; other PCI functions are enumeration-only, including virtio devices.
No BAR sizing, MSI, virtio driver, ECAM, hotplug, or resource assignment is provided.

Boot logs show at most 32 present functions (including non-AHCI vendor/device IDs)
and a total/omitted count; absent slots produce no logs.

## Host tests

Run synthetic config header tests without QEMU or kernel I/O:

```bash
cargo test -Z bindeps --locked -p krust --test pci
```

## QEMU inspection

To inspect an additional modern virtio RNG function without binding a driver,
use a directory under `target/` for the runner's generated files and trace:

```bash
mkdir -p target/pci-probe
TMPDIR="$PWD/target/pci-probe" timeout --kill-after=2s 30s \
  target/debug/krust run bios --headless --vhd target/krust-disk-ext4.vhd -- \
  -snapshot -no-reboot -device virtio-rng-pci,disable-legacy=on
```

The trace should contain `vendor=1af4 device=1044` with `ahci=false`, alongside
the existing AHCI controller. The timeout bounds this inspection and `-snapshot`
keeps the source VHD unchanged.
