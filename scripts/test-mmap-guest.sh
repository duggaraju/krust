#!/usr/bin/env bash
# Add the static fixture only to an owned ext4 copy; boot it using the smoke harness.
set -euo pipefail
cd "$(dirname "$0")/.."
if (( $# > 2 )); then
    echo "Usage: $0 [ext4-disk.vhd [runner]]" >&2
    exit 2
fi
for tool in cc debugfs realpath sha256sum; do
    command -v "$tool" >/dev/null || { echo "Missing required tool: $tool" >&2; exit 2; }
done
disk=$(realpath -e "${1:-target/krust-disk-ext4.vhd}")
runner=$(realpath -e "${2:-target/debug/krust}")
[[ -f "$disk" && -r "$disk" ]] || { echo "VHD must be a readable file" >&2; exit 2; }
[[ -f "$runner" && -x "$runner" ]] || { echo "Runner must be executable" >&2; exit 2; }
before=$(sha256sum "$disk")
mkdir -p target
work="$PWD/target/mmap-guest-$$"
mkdir "$work"
cleanup() {
    status=$?
    trap - EXIT
    if ! after=$(sha256sum "$disk") || [[ "$after" != "$before" ]]; then
        echo "Source VHD changed!" >&2
        status=1
    else
        echo "Source VHD unchanged"
    fi
    rm -rf -- "$work"
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
export TMPDIR="$work"
cc -std=c11 -O2 -Wall -Wextra -Werror -ffreestanding -nostdlib -static -no-pie \
    -fno-pie -fno-stack-protector -mno-red-zone -Wl,--build-id=none \
    scripts/tests/mmap-guest.c -o "$work/probe"
cc -std=c11 -O2 -Wall -Wextra -Werror -ffreestanding -nostdlib -static -no-pie \
    -fno-pie -fno-stack-protector -mno-red-zone -Wl,--build-id=none \
    scripts/tests/mmap-protection-guest.c -o "$work/protection"
cp --reflink=auto "$disk" "$work/disk.vhd"
debugfs -w -R 'rm /bin/krust-mmap-probe' "$work/disk.vhd" \
    > "$work/debugfs.log" 2>&1
debugfs -w -R "write $work/probe /bin/krust-mmap-probe" "$work/disk.vhd" \
    >> "$work/debugfs.log" 2>&1
debugfs -w -R 'set_inode_field /bin/krust-mmap-probe mode 0100755' "$work/disk.vhd" \
    >> "$work/debugfs.log" 2>&1
debugfs -R 'stat /bin/krust-mmap-probe' "$work/disk.vhd" 2>/dev/null |
    grep -q 'Type: regular'
debugfs -w -R 'rm /bin/krust-mmap-protection' "$work/disk.vhd" \
    >> "$work/debugfs.log" 2>&1
debugfs -w -R "write $work/protection /bin/krust-mmap-protection" "$work/disk.vhd" \
    >> "$work/debugfs.log" 2>&1
debugfs -w -R 'set_inode_field /bin/krust-mmap-protection mode 0100755' "$work/disk.vhd" \
    >> "$work/debugfs.log" 2>&1
debugfs -R 'stat /bin/krust-mmap-protection' "$work/disk.vhd" 2>/dev/null |
    grep -q 'Type: regular'
bash scripts/test-mmap-protections.sh "$work/disk.vhd" "$runner"
KRUST_SMOKE_MMAP_PROBE=1 scripts/smoke-userspace.sh "$work/disk.vhd" "$runner"
for mode in bios uefi; do
    cp "target/userspace-smoke-$mode.log" "target/mmap-guest-$mode.log"
    cp "target/userspace-smoke-$mode-kernel.log" "target/mmap-guest-$mode-kernel.log"
done
