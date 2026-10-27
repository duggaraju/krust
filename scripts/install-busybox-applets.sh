#!/usr/bin/env bash
# Install applets from an already built native BusyBox; no download or rebuild.
set -euo pipefail
if (( $# != 3 )) || [[ "$3" != fat && "$3" != ext4 ]]; then
    echo "Usage: $0 <busybox> <rootfs> <fat|ext4>" >&2
    exit 2
fi
binary=$(realpath -e "$1")
root=$2
format=$3
applets=$("$binary" --list)
grep -qx sh <<< "$applets" || { echo "BusyBox must include the sh applet" >&2; exit 1; }
mkdir -p "$root/bin"
if [[ "$binary" != "$(realpath -m "$root/bin/busybox")" ]]; then
    rm -f -- "$root/bin/busybox"
    cp "$binary" "$root/bin/busybox"
fi
chmod +x "$root/bin/busybox"
for tool in ash sh bash ls cat echo cp mv rm mkdir rmdir ln touch \
    grep sed awk cut sort uniq head tail wc find xargs stat chmod chown \
    ps kill env printenv date sleep true false hexdump od strings gzip gunzip tar wget; do
    rm -f -- "$root/bin/$tool"
    # Do not advertise disabled or nonexistent applets (e.g. bash).
    grep -qx "$tool" <<< "$applets" || continue
    if [[ "$format" == ext4 ]]; then
        ln -s /bin/busybox "$root/bin/$tool"
    elif [[ "$tool" == sh || "$tool" == ash ]]; then
        # FAT has no links: shell entrypoints must be real executables.
        cp "$binary" "$root/bin/$tool"
        chmod +x "$root/bin/$tool"
    else
        printf '#!/bin/busybox sh\nexec /bin/busybox %s "$@"\n' "$tool" > "$root/bin/$tool"
        chmod +x "$root/bin/$tool"
    fi
done
