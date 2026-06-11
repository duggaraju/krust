#!/usr/bin/env bash
# setup-busybox.sh — Build a static BusyBox rootfs and pack it into a VHD.
#
# Usage:
#   ./scripts/setup-busybox.sh [rootfs_dir] [output.vhd]
#
# Defaults:
#   rootfs_dir = target/rootfs
#   output.vhd = target/krust-disk.vhd
#
# Requirements (install once):
#   apt install build-essential wget libncurses-dev
#
# The generated VHD can be passed to the launcher:
#   cargo run -Z bindeps -- run uefi --vhd target/krust-disk.vhd

set -euo pipefail

ROOTFS="${1:-target/rootfs}"
OUTPUT="${2:-target/krust-disk.vhd}"
BUSYBOX_VERSION="${BUSYBOX_VERSION:-1.36.1}"
BUSYBOX_URL="https://busybox.net/downloads/busybox-${BUSYBOX_VERSION}.tar.bz2"
BUILD_DIR="target/busybox-build"

# ── helpers ──────────────────────────────────────────────────────────────────
info()  { printf '\e[1;32m[+]\e[0m %s\n' "$*"; }
warn()  { printf '\e[1;33m[!]\e[0m %s\n' "$*"; }
die()   { printf '\e[1;31m[✗]\e[0m %s\n' "$*" >&2; exit 1; }

require() {
    for cmd in "$@"; do
        command -v "$cmd" &>/dev/null || die "Required tool not found: $cmd  (apt install build-essential wget libncurses-dev)"
    done
}

# ── 1. Check prerequisites ────────────────────────────────────────────────────
require wget tar make gcc

# ── 2. Download BusyBox source ────────────────────────────────────────────────
TARBALL="target/busybox-${BUSYBOX_VERSION}.tar.bz2"
mkdir -p target

if [[ ! -f "$TARBALL" ]]; then
    info "Downloading BusyBox ${BUSYBOX_VERSION}..."
    wget -q --show-progress -O "$TARBALL" "$BUSYBOX_URL"
else
    info "Using cached tarball: $TARBALL"
fi

# ── 3. Extract and configure ──────────────────────────────────────────────────
if [[ ! -d "$BUILD_DIR" ]]; then
    info "Extracting BusyBox..."
    mkdir -p "$BUILD_DIR"
    tar -xjf "$TARBALL" -C "$BUILD_DIR" --strip-components=1
fi

info "Configuring BusyBox (static, defconfig)..."
make -C "$BUILD_DIR" defconfig > /dev/null

# Force static linking
sed -i 's/# CONFIG_STATIC is not set/CONFIG_STATIC=y/' "$BUILD_DIR/.config"
# Disable features that don't link cleanly in fully static builds
sed -i 's/CONFIG_TC=y/# CONFIG_TC is not set/' "$BUILD_DIR/.config"          || true
sed -i 's/CONFIG_NSLOOKUP=y/# CONFIG_NSLOOKUP is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_HOST=y/# CONFIG_HOST is not set/' "$BUILD_DIR/.config"       || true

# ── 4. Build ──────────────────────────────────────────────────────────────────
NPROC=$(nproc 2>/dev/null || echo 4)
info "Building BusyBox (${NPROC} jobs) — this takes ~1 minute..."
make -C "$BUILD_DIR" -j"${NPROC}" LDFLAGS="--static" 2>&1 | tail -5

BUSYBOX_BIN="$BUILD_DIR/busybox"
[[ -f "$BUSYBOX_BIN" ]] || die "Build failed: $BUSYBOX_BIN not found"
info "Built: $BUSYBOX_BIN ($(du -sh "$BUSYBOX_BIN" | cut -f1))"

# ── 5. Populate rootfs ────────────────────────────────────────────────────────
info "Populating rootfs at $ROOTFS..."
mkdir -p \
    "$ROOTFS/bin"   \
    "$ROOTFS/sbin"  \
    "$ROOTFS/usr/bin" \
    "$ROOTFS/usr/sbin" \
    "$ROOTFS/lib"   \
    "$ROOTFS/tmp"   \
    "$ROOTFS/etc"   \
    "$ROOTFS/proc"  \
    "$ROOTFS/dev"

# Install the busybox binary
cp "$BUSYBOX_BIN" "$ROOTFS/bin/busybox"
chmod +x "$ROOTFS/bin/busybox"

# Create symlinks for common tools in /bin
TOOLS=(
    ash sh bash
    ls cat echo cp mv rm mkdir rmdir ln touch
    grep sed awk cut sort uniq head tail wc
    find xargs
    stat chmod chown
    mount umount
    ps kill
    env printenv
    date sleep true false
    hexdump od strings
    gzip gunzip tar
    wget
)

# Create small wrapper scripts for each tool — FAT has no symlinks
# Each script is ~40 bytes; busybox binary is stored only once.
for tool in "${TOOLS[@]}"; do
    [[ "$tool" == "busybox" ]] && continue
    printf '#!/bin/sh\nexec /bin/busybox %s "$@"\n' "$tool" > "$ROOTFS/bin/$tool"
    chmod +x "$ROOTFS/bin/$tool"
done

# /sbin/init wrapper
printf '#!/bin/sh\nexec /bin/busybox sh "$@"\n' > "$ROOTFS/sbin/init"
chmod +x "$ROOTFS/sbin/init"

# Minimal /etc/passwd and /etc/group
cat > "$ROOTFS/etc/passwd" <<'EOF'
root:x:0:0:root:/root:/bin/sh
EOF
cat > "$ROOTFS/etc/group" <<'EOF'
root:x:0:
EOF

info "Rootfs ready:"
find "$ROOTFS" -maxdepth 2 | sort | sed 's/^/  /'

# ── 6. Pack into VHD ──────────────────────────────────────────────────────────
info "Packing into VHD: $OUTPUT"
# Exclude virtual filesystem directories (proc, dev) — they're mounted separately at runtime
cargo run -Z bindeps --quiet -- mkdisk "$ROOTFS" "$OUTPUT" --size-mib 64 --exclude proc,dev

info "Done!  $(du -sh "$OUTPUT" | cut -f1)  →  $OUTPUT"
info ""
info "To run:  cargo run -Z bindeps -- run uefi --vhd $OUTPUT"
