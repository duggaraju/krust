#!/usr/bin/env bash
# setup-busybox.sh — Build a static BusyBox rootfs and pack it into a VHD.
#
# Usage:
#   ./scripts/setup-busybox.sh [rootfs_dir] [output.vhd] [--format fat|ext4] [--libc musl|glibc]
#
# Defaults:
#   rootfs_dir = target/rootfs
#   output.vhd = target/krust-disk-<format>.vhd  (e.g. krust-disk-fat.vhd)
#   format     = fat   (set DISK_FORMAT=ext4 or pass --format ext4)
#   libc       = musl  (set BUSYBOX_LIBC=glibc or pass --libc glibc)
#
# Requirements (install once):
#   apt install build-essential wget libncurses-dev musl-tools
#   # For ext4 support also install:
#   apt install e2fsprogs
#
# The generated VHD can be passed to the launcher:
#   cargo run -Z bindeps -- run uefi --vhd target/krust-disk-fat.vhd
#   cargo run -Z bindeps -- run uefi --vhd target/krust-disk-ext4.vhd

set -euo pipefail

# ── argument parsing ──────────────────────────────────────────────────────────
# Options can appear in any position relative to positional args.
DISK_FORMAT="${DISK_FORMAT:-fat}"
BUSYBOX_LIBC="${BUSYBOX_LIBC:-musl}"
_positional=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --format)
            DISK_FORMAT="${2:?'--format requires fat or ext4'}"
            shift 2
            ;;
        --format=*)
            DISK_FORMAT="${1#--format=}"
            shift
            ;;
        --libc)
            BUSYBOX_LIBC="${2:?'--libc requires musl or glibc'}"
            shift 2
            ;;
        --libc=*)
            BUSYBOX_LIBC="${1#--libc=}"
            shift
            ;;
        -*)
            echo "Unknown option: $1" >&2; exit 1
            ;;
        *)
            _positional+=("$1")
            shift
            ;;
    esac
done

ROOTFS="${_positional[0]:-target/rootfs}"
# Default output name includes the format so fat and ext4 images don't collide.
OUTPUT="${_positional[1]:-target/krust-disk-${DISK_FORMAT}.vhd}"

if [[ "$DISK_FORMAT" != "fat" && "$DISK_FORMAT" != "ext4" ]]; then
    echo "error: --format must be 'fat' or 'ext4'" >&2; exit 1
fi
if [[ "$BUSYBOX_LIBC" != "musl" && "$BUSYBOX_LIBC" != "glibc" ]]; then
    echo "error: --libc must be 'musl' or 'glibc'" >&2; exit 1
fi

BUSYBOX_VERSION="${BUSYBOX_VERSION:-1.36.1}"
BUSYBOX_URL="https://busybox.net/downloads/busybox-${BUSYBOX_VERSION}.tar.bz2"
BUILD_DIR="target/busybox-build"

# ── helpers ──────────────────────────────────────────────────────────────────
info()  { printf '\e[1;32m[+]\e[0m %s\n' "$*"; }
warn()  { printf '\e[1;33m[!]\e[0m %s\n' "$*"; }
die()   { printf '\e[1;31m[✗]\e[0m %s\n' "$*" >&2; exit 1; }

require() {
    for cmd in "$@"; do
        command -v "$cmd" &>/dev/null || die "Required tool not found: $cmd  (apt install build-essential wget libncurses-dev musl-tools)"
    done
}

# ── 1. Check prerequisites ────────────────────────────────────────────────────
require wget tar make gcc
if [[ "$BUSYBOX_LIBC" == "musl" ]]; then
    if command -v musl-gcc &>/dev/null; then
        BUSYBOX_CC="musl-gcc"
    elif command -v x86_64-linux-musl-gcc &>/dev/null; then
        BUSYBOX_CC="x86_64-linux-musl-gcc"
    else
        die "No musl C compiler found (install musl-tools or musl-cross-make; apt install musl-tools)"
    fi
else
    BUSYBOX_CC="${CC:-gcc}"
fi
if [[ "$DISK_FORMAT" == "ext4" ]]; then
    command -v mkfs.ext4 &>/dev/null || die "mkfs.ext4 not found — install e2fsprogs:  apt install e2fsprogs"
fi

info "Disk format: $DISK_FORMAT"
info "BusyBox libc: $BUSYBOX_LIBC"

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

BUSYBOX_HOSTCC="${HOSTCC:-gcc}"

info "Configuring BusyBox (static, defconfig) using ${BUSYBOX_CC}..."
make -C "$BUILD_DIR" defconfig > /dev/null

# Force static linking
sed -i 's/# CONFIG_STATIC is not set/CONFIG_STATIC=y/' "$BUILD_DIR/.config"
# Disable features that don't link cleanly in fully static builds
sed -i 's/CONFIG_TC=y/# CONFIG_TC is not set/' "$BUILD_DIR/.config"          || true
sed -i 's/CONFIG_NSLOOKUP=y/# CONFIG_NSLOOKUP is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_HOST=y/# CONFIG_HOST is not set/' "$BUILD_DIR/.config"       || true
# Disable init/VT applets that pull in Linux console headers.
sed -i 's/CONFIG_INIT=y/# CONFIG_INIT is not set/' "$BUILD_DIR/.config"       || true
sed -i 's/CONFIG_LINUXRC=y/# CONFIG_LINUXRC is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_FEATURE_USE_INITTAB=y/# CONFIG_FEATURE_USE_INITTAB is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_GETTY=y/# CONFIG_GETTY is not set/' "$BUILD_DIR/.config"     || true
sed -i 's/CONFIG_OPENVT=y/# CONFIG_OPENVT is not set/' "$BUILD_DIR/.config"   || true
sed -i 's/CONFIG_CHVT=y/# CONFIG_CHVT is not set/' "$BUILD_DIR/.config"       || true
sed -i 's/CONFIG_DEALLOCVT=y/# CONFIG_DEALLOCVT is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_SETCONSOLE=y/# CONFIG_SETCONSOLE is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_SETSID=y/# CONFIG_SETSID is not set/' "$BUILD_DIR/.config"   || true
sed -i 's/CONFIG_RUN_INIT=y/# CONFIG_RUN_INIT is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_CONSPY=y/# CONFIG_CONSPY is not set/' "$BUILD_DIR/.config"     || true
sed -i 's/CONFIG_BEEP=y/# CONFIG_BEEP is not set/' "$BUILD_DIR/.config"         || true
sed -i 's/CONFIG_FBSPLASH=y/# CONFIG_FBSPLASH is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_FBSET=y/# CONFIG_FBSET is not set/' "$BUILD_DIR/.config"       || true
sed -i 's/CONFIG_I2CGET=y/# CONFIG_I2CGET is not set/' "$BUILD_DIR/.config"     || true
sed -i 's/CONFIG_I2CSET=y/# CONFIG_I2CSET is not set/' "$BUILD_DIR/.config"     || true
sed -i 's/CONFIG_I2CDUMP=y/# CONFIG_I2CDUMP is not set/' "$BUILD_DIR/.config"    || true
sed -i 's/CONFIG_I2CDETECT=y/# CONFIG_I2CDETECT is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_I2CTRANSFER=y/# CONFIG_I2CTRANSFER is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_LOSETUP=y/# CONFIG_LOSETUP is not set/' "$BUILD_DIR/.config"   || true
sed -i 's/CONFIG_MOUNT=y/# CONFIG_MOUNT is not set/' "$BUILD_DIR/.config"       || true
sed -i 's/CONFIG_MOUNTPOINT=y/# CONFIG_MOUNTPOINT is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_FEATURE_MOUNT_LOOP=y/# CONFIG_FEATURE_MOUNT_LOOP is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_FEATURE_MOUNT_LOOP_CREATE=y/# CONFIG_FEATURE_MOUNT_LOOP_CREATE is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_SWAPON=y/# CONFIG_SWAPON is not set/' "$BUILD_DIR/.config"     || true
sed -i 's/CONFIG_SWAPOFF=y/# CONFIG_SWAPOFF is not set/' "$BUILD_DIR/.config"   || true
sed -i 's/CONFIG_PARTPROBE=y/# CONFIG_PARTPROBE is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_FDISK=y/# CONFIG_FDISK is not set/' "$BUILD_DIR/.config"       || true
# Disable console applets that need linux/kd.h on this toolchain.
sed -i 's/CONFIG_KBD_MODE=y/# CONFIG_KBD_MODE is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_LOADFONT=y/# CONFIG_LOADFONT is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_SETFONT=y/# CONFIG_SETFONT is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_SHOWKEY=y/# CONFIG_SHOWKEY is not set/' "$BUILD_DIR/.config" || true
# Disable netlink-based helpers that pull in linux/netlink.h.
sed -i 's/CONFIG_IFPLUGD=y/# CONFIG_IFPLUGD is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_UEVENT=y/# CONFIG_UEVENT is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_MDEV=y/# CONFIG_MDEV is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_FEATURE_MDEV_CONF=y/# CONFIG_FEATURE_MDEV_CONF is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_FEATURE_MDEV_DAEMON=y/# CONFIG_FEATURE_MDEV_DAEMON is not set/' "$BUILD_DIR/.config" || true
# Disable networking applets we do not need in the kernel dev loop.
sed -i 's/CONFIG_ARP=y/# CONFIG_ARP is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_ARPING=y/# CONFIG_ARPING is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_BRCTL=y/# CONFIG_BRCTL is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_IFCONFIG=y/# CONFIG_IFCONFIG is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_IP=y/# CONFIG_IP is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_IPADDR=y/# CONFIG_IPADDR is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_IPLINK=y/# CONFIG_IPLINK is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_IPROUTE=y/# CONFIG_IPROUTE is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_IPTUNNEL=y/# CONFIG_IPTUNNEL is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_IPRULE=y/# CONFIG_IPRULE is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_IPNEIGH=y/# CONFIG_IPNEIGH is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_IPCALC=y/# CONFIG_IPCALC is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_FAKEIDENTD=y/# CONFIG_FAKEIDENTD is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_ETHER_WAKE=y/# CONFIG_ETHER_WAKE is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_FTPD=y/# CONFIG_FTPD is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_HTTPD=y/# CONFIG_HTTPD is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_NTPD=y/# CONFIG_NTPD is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_TFTP=y/# CONFIG_TFTP is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_TFTPD=y/# CONFIG_TFTPD is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_UDHCPD=y/# CONFIG_UDHCPD is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_UDHCPC=y/# CONFIG_UDHCPC is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_UDHCPC6=y/# CONFIG_UDHCPC6 is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_FEATURE_IFUPDOWN_IP=y/# CONFIG_FEATURE_IFUPDOWN_IP is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_FEATURE_IFUPDOWN_IPV4=y/# CONFIG_FEATURE_IFUPDOWN_IPV4 is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_FEATURE_IFUPDOWN_IPV6=y/# CONFIG_FEATURE_IFUPDOWN_IPV6 is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_FEATURE_UDHCP_RFC3397=y/# CONFIG_FEATURE_UDHCP_RFC3397 is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_NAMEIF=y/# CONFIG_NAMEIF is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_NC=y/# CONFIG_NC is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_NC_SERVER=y/# CONFIG_NC_SERVER is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_NC_EXTRA=y/# CONFIG_NC_EXTRA is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_NC_110_COMPAT=y/# CONFIG_NC_110_COMPAT is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_PING=y/# CONFIG_PING is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_PING6=y/# CONFIG_PING6 is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_ROUTE=y/# CONFIG_ROUTE is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_TCPSVD=y/# CONFIG_TCPSVD is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_TELNET=y/# CONFIG_TELNET is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_TELNETD=y/# CONFIG_TELNETD is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_TRACEROUTE=y/# CONFIG_TRACEROUTE is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_TRACEROUTE6=y/# CONFIG_TRACEROUTE6 is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_VCONFIG=y/# CONFIG_VCONFIG is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_WGET=y/# CONFIG_WGET is not set/' "$BUILD_DIR/.config" || true
# Disable disk utils that pull in unavailable kernel UAPI headers.
sed -i 's/CONFIG_HDPARM=y/# CONFIG_HDPARM is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_RAIDAUTORUN=y/# CONFIG_RAIDAUTORUN is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_NANDWRITE=y/# CONFIG_NANDWRITE is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_NANDDUMP=y/# CONFIG_NANDDUMP is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_SEEDRNG=y/# CONFIG_SEEDRNG is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_UBIATTACH=y/# CONFIG_UBIATTACH is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_UBIDETACH=y/# CONFIG_UBIDETACH is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_UBIMKVOL=y/# CONFIG_UBIMKVOL is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_UBIRMVOL=y/# CONFIG_UBIRMVOL is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_UBIRSVOL=y/# CONFIG_UBIRSVOL is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_UBIUPDATEVOL=y/# CONFIG_UBIUPDATEVOL is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_UBIRENAME=y/# CONFIG_UBIRENAME is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_WATCHDOG=y/# CONFIG_WATCHDOG is not set/' "$BUILD_DIR/.config" || true
# Disable loginutils applets that need VT headers.
sed -i 's/CONFIG_VLOCK=y/# CONFIG_VLOCK is not set/' "$BUILD_DIR/.config" || true
# Disable capability helpers that need linux/capability.h.
sed -i 's/CONFIG_FEATURE_SETPRIV_CAPABILITIES=y/# CONFIG_FEATURE_SETPRIV_CAPABILITIES is not set/' "$BUILD_DIR/.config" || true
sed -i 's/CONFIG_FEATURE_SETPRIV_CAPABILITY_NAMES=y/# CONFIG_FEATURE_SETPRIV_CAPABILITY_NAMES is not set/' "$BUILD_DIR/.config" || true

# ── 4. Build ──────────────────────────────────────────────────────────────────
NPROC=$(nproc 2>/dev/null || echo 4)
info "Building BusyBox (${NPROC} jobs) — this takes ~1 minute..."
make -C "$BUILD_DIR" clean >/dev/null
make -C "$BUILD_DIR" -j"${NPROC}" CC="${BUSYBOX_CC}" HOSTCC="${BUSYBOX_HOSTCC}" \
    CFLAGS="-idirafter /usr/include -idirafter /usr/include/x86_64-linux-gnu" LDFLAGS="--static" 2>&1 | tail -5

BUSYBOX_BIN="$BUILD_DIR/busybox"
[[ -f "$BUSYBOX_BIN" ]] || die "Build failed: $BUSYBOX_BIN not found"
info "Built: $BUSYBOX_BIN ($(du -sh "$BUSYBOX_BIN" | cut -f1))"

# ── 5. Populate rootfs ────────────────────────────────────────────────────────
info "Populating rootfs at $ROOTFS..."
rm -rf \
    "$ROOTFS/bin"   \
    "$ROOTFS/sbin"  \
    "$ROOTFS/usr"   \
    "$ROOTFS/lib"   \
    "$ROOTFS/tmp"   \
    "$ROOTFS/etc"   \
    "$ROOTFS/proc"  \
    "$ROOTFS/dev"
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

# Tools to expose from busybox
TOOLS=(
    ash sh bash
    ls cat echo cp mv rm mkdir rmdir ln touch
    grep sed awk cut sort uniq head tail wc
    find xargs
    stat chmod chown
    ps kill
    env printenv
    date sleep true false
    hexdump od strings
    gzip gunzip tar
    wget
)

if [[ "$DISK_FORMAT" == "ext4" ]]; then
    # ext4 supports symlinks — use the canonical busybox multi-call approach.
    info "Creating symlinks for busybox applets..."
    for tool in "${TOOLS[@]}"; do
        [[ "$tool" == "busybox" ]] && continue
        ln -sf /bin/busybox "$ROOTFS/bin/$tool"
    done
    # /sbin/init → busybox sh
    ln -sf /bin/busybox "$ROOTFS/sbin/init"
else
    # FAT has no symlinks — use small wrapper scripts instead.
    info "Creating wrapper scripts for busybox applets (FAT has no symlinks)..."
    for tool in "${TOOLS[@]}"; do
        [[ "$tool" == "busybox" ]] && continue
        printf '#!/bin/sh\nexec /bin/busybox %s "$@"\n' "$tool" > "$ROOTFS/bin/$tool"
        chmod +x "$ROOTFS/bin/$tool"
    done
    # /sbin/init wrapper
    printf '#!/bin/sh\nexec /bin/busybox sh "$@"\n' > "$ROOTFS/sbin/init"
    chmod +x "$ROOTFS/sbin/init"
fi

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
info "Packing into VHD: $OUTPUT  (format: $DISK_FORMAT)"
# Exclude virtual filesystem directories (proc, dev) — they're mounted separately at runtime
cargo run -Z bindeps --quiet -- mkdisk "$ROOTFS" "$OUTPUT" --size-mib 64 --exclude proc,dev --format "$DISK_FORMAT"

info "Done!  $(du -sh "$OUTPUT" | cut -f1)  →  $OUTPUT"
info ""
info "To run:  cargo run -Z bindeps -- run uefi --vhd $OUTPUT"
