#!/usr/bin/env bash
# Real QEMU coverage for generated/custom fstab and early root selection.
set -euo pipefail
cd "$(dirname "$0")/.."
if (( $# > 2 )); then
    echo "Usage: $0 [busybox [runner]]" >&2
    exit 2
fi
busybox=$(realpath -e "${1:-target/rootfs/bin/busybox}")
runner=$(realpath -e "${2:-target/debug/krust}")
for tool in debugfs mkfs.ext4 setsid timeout sha256sum; do
    command -v "$tool" >/dev/null || { echo "Missing required tool: $tool" >&2; exit 2; }
done
work="$PWD/target/boot-mounts-$$"
mkdir "$work"
active=
cleanup() {
    status=$?
    trap - EXIT
    if [[ -n "$active" ]]; then
        kill -TERM -- "-$active" 2>/dev/null || true
        sleep 0.2
        kill -KILL -- "-$active" 2>/dev/null || true
        wait "$active" 2>/dev/null || true
    fi
    rm -rf -- "$work"
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
mkdir -p "$work/root/bin" "$work/root/etc"
cp "$busybox" "$work/root/bin/busybox"
for format in fat ext4; do
    cat > "$work/$format.fstab" <<EOF
# Deliberately list the nested mount first to test parent ordering.
/dev/sda / $format defaults 0 0
none /scratch/child ramfs defaults 0 0
none /scratch ramfs defaults 0 0
/dev/not-present /offline fat noauto 0 0
proc /proc proc defaults 0 0
dev /dev dev defaults 0 0
EOF
    options=()
    if [[ "$format" == fat ]]; then
        cp "$work/$format.fstab" "$work/root/etc/fstab"
    else
        options=(--fstab "$work/$format.fstab")
    fi
    source_hash=$(sha256sum "$work/root/etc/fstab")
    "$runner" mkdisk "$work/root" "$work/$format.vhd" --format "$format" "${options[@]}" \
        > "target/boot-mounts-$format-image.log" 2>&1
    [[ "$(sha256sum "$work/root/etc/fstab")" == "$source_hash" ]]
    grep -qx "root_fs = \"$format\"" "$work/$format.vhd.boot.toml"
    scripts/smoke-userspace.sh "$work/$format.vhd" "$runner"
    for mode in bios uefi; do
        trace="target/userspace-smoke-$mode-kernel.log"
        grep -Fq "mount: mounted filesystem '$format' on '/'" "$trace"
        grep -Fq "fstab: root entry matches early boot settings" "$trace"
        grep -Fq "fstab: skipping noauto mount '/offline'" "$trace"
        parent=$(grep -nF "mount: mounted filesystem 'ramfs' on '/scratch'" "$trace" | cut -d: -f1)
        child=$(grep -nF "mount: mounted filesystem 'ramfs' on '/scratch/child'" "$trace" | cut -d: -f1)
        [[ -n "$parent" && -n "$child" && "$parent" -lt "$child" ]]
        cp "$trace" "target/boot-mounts-$format-$mode-kernel.log"
    done
    echo "$format generated/custom fstab root passed on BIOS and UEFI"
done

expect_boot_failure() {
    local mode=$1 case_name=$2 expected=$3
    shift 3
    local log="$PWD/target/boot-mounts-$mode-$case_name.log" trace status=0
    setsid timeout --signal=TERM --kill-after=2s 120s \
        env TMPDIR="$work" "$runner" run "$mode" --headless "$@" -- -snapshot -no-reboot \
        > "$log" 2>&1 &
    active=$!
    wait "$active" || status=$?
    active=
    [[ "$status" == 1 ]] || { echo "$mode $case_name: expected guest failure, got $status" >&2; return 1; }
    grep -Fq 'guest 0x11' "$log"
    trace=$(sed -n 's/^headless kernel trace log: //p' "$log" | tr -d '\r')
    [[ "$trace" == "$work"/krust-trace-*.log && -f "$trace" ]]
    grep -Fq 'boot mounts failed:' "$trace"
    grep -Fq "$expected" "$trace"
    ! grep -Fq 'fstab: configured mounts ready' "$trace"
    cp "$trace" "target/boot-mounts-$mode-$case_name-kernel.log"
    echo "$mode $case_name rejected explicitly"
}
cat > "$work/missing-root.toml" <<'EOF'
root_fs = "ext4"
root_device = "/dev/not-present"
EOF
for mode in bios uefi; do
    expect_boot_failure "$mode" missing-root 'root ext4 on /dev/not-present: NotFound' \
        --no-vhd --boot-config "$work/missing-root.toml"
done
cp "$work/ext4.vhd" "$work/invalid.vhd"
for case_name in option duplicate mismatch missing-mount malformed; do
    case "$case_name" in
        option) printf 'proc /proc proc ro 0 0\n' > "$work/invalid.fstab"; expected='unsupported mount option' ;;
        duplicate) printf 'proc /proc proc defaults 0 0\nproc /proc proc defaults 0 0\n' > "$work/invalid.fstab"; expected='duplicate mount target' ;;
        mismatch) printf '/dev/other / ext4 defaults 0 0\n' > "$work/invalid.fstab"; expected='fstab root entry disagrees' ;;
        missing-mount) printf '/dev/not-present /offline fat defaults 0 0\n' > "$work/invalid.fstab"; expected='fstab: /dev/not-present on /offline (fat): NotFound' ;;
        malformed) printf 'proc /proc proc defaults\n' > "$work/invalid.fstab"; expected='expected six fields' ;;
    esac
    printf 'rm /etc/fstab\nwrite "%s" /etc/fstab\n' "$work/invalid.fstab" > "$work/debugfs.commands"
    debugfs -w -f "$work/debugfs.commands" "$work/invalid.vhd" > "$work/debugfs.log" 2>&1
    debugfs -R 'cat /etc/fstab' "$work/invalid.vhd" 2>/dev/null > "$work/readback"
    cmp "$work/invalid.fstab" "$work/readback"
    hash=$(sha256sum "$work/invalid.vhd")
    for mode in bios uefi; do
        expect_boot_failure "$mode" "$case_name" "$expected" --vhd "$work/invalid.vhd"
    done
    [[ "$(sha256sum "$work/invalid.vhd")" == "$hash" ]]
done
printf 'root_fs = "unknown"\n' > "$work/unknown.toml"
if "$runner" run bios --no-vhd --boot-config "$work/unknown.toml" > "$work/unknown.log" 2>&1; then
    echo "unknown root filesystem was accepted" >&2
    exit 1
fi
grep -Fq 'unknown setting or invalid value' "$work/unknown.log"
! grep -Fq 'headless kernel trace log:' "$work/unknown.log"
echo "Boot mount configuration tests passed"
