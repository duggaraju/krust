#!/usr/bin/env bash
# Inject only into an owned copy, then require actual BIOS/UEFI guest markers.
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
work="$PWD/target/cloexec-guest-$$"
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
for image in before after; do
    flags=()
    [[ "$image" != after ]] || flags+=(-DAFTER_EXEC)
    cc -std=c11 -O2 -Wall -Wextra -Werror -ffreestanding -nostdlib -static -no-pie \
        -fno-pie -fno-stack-protector -mno-red-zone -Wl,--build-id=none \
        "${flags[@]}" scripts/tests/cloexec-guest.c -o "$work/$image"
done
printf 'abcdef\n' > "$work/data"
cp --reflink=auto "$disk" "$work/disk.vhd"
for image in before after data; do
    path="/bin/krust-cloexec-$image"
    debugfs -w -R "rm $path" "$work/disk.vhd" >> "$work/debugfs.log" 2>&1
    debugfs -w -R "write $work/$image $path" "$work/disk.vhd" >> "$work/debugfs.log" 2>&1
    debugfs -w -R "set_inode_field $path mode 0100755" "$work/disk.vhd" >> "$work/debugfs.log" 2>&1
    debugfs -R "stat $path" "$work/disk.vhd" 2>/dev/null | grep -q 'Type: regular'
done
KRUST_SMOKE_CLOEXEC_PROBE=1 bash scripts/smoke-userspace.sh "$work/disk.vhd" "$runner"
for mode in bios uefi; do
    cp "target/userspace-smoke-$mode.log" "target/cloexec-guest-$mode.log"
    cp "target/userspace-smoke-$mode-kernel.log" "target/cloexec-guest-$mode-kernel.log"
done
