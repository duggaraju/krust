#!/usr/bin/env bash
# Exercise the real ring-3/IRQ entry path against an existing BusyBox ext4 VHD.
set -euo pipefail
cd "$(dirname "$0")/.."

disk=$(realpath "${1:-target/krust-disk-ext4.vhd}")
runner=$(realpath "${2:-target/debug/krust}")
work="$PWD/target/userspace-smoke-$$"
mkdir "$work"
before=$(sha256sum "$disk")
active=
cleanup() {
    if [[ -n "$active" ]]; then
        kill -TERM "$active" 2>/dev/null || true
        wait "$active" 2>/dev/null || true
    fi
    rm -rf "$work"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

for mode in bios uefi; do
    log="$PWD/target/userspace-smoke-$mode.log"
    commands=(
        "/bin/busybox echo KRUST_ECHO_OK"
        "/bin/busybox ls /proc /dev"
        "/bin/busybox true"
        "/bin/busybox echo KRUST_FORK_EXEC_OK"
        "exit"
    )
    : > "$log"
    coproc GUEST {
        exec timeout --signal=TERM --kill-after=5s 120s \
            env TMPDIR="$work" "$runner" run "$mode" --headless --vhd "$disk" -- \
            -snapshot -no-reboot 2>&1
    }
    active=$GUEST_PID
    exec {input}>&"${GUEST[1]}"
    exec {output}<&"${GUEST[0]}"
    next=0
    suffix=
    while IFS= read -r -N 1 char <&"$output"; do
        printf '%s' "$char" >> "$log"
        suffix="${suffix}${char}"
        if [[ ${#suffix} -gt 8 ]]; then
            suffix="${suffix: -8}"
        fi
        if [[ "$suffix" == *"krust> " && $next -lt ${#commands[@]} ]]; then
            printf '%s\n' "${commands[$next]}" >&"$input"
            next=$((next + 1))
            suffix=
        fi
    done
    status=0
    wait "$active" || status=$?
    active=
    exec {input}>&-
    exec {output}<&-
    trace=$(sed -n 's/^headless kernel trace log: //p' "$log" | tr -d '\r')
    if [[ -f "$trace" ]]; then
        cp "$trace" "target/userspace-smoke-$mode-kernel.log"
    fi
    if [[ $status != 0 || $next != ${#commands[@]} || ! -f "$trace" ]] ||
        ! grep -Eq $'(^|krust> )KRUST_ECHO_OK\r?$' "$log" ||
        ! grep -Eq $'(^|krust> )KRUST_FORK_EXEC_OK\r?$' "$log" ||
        ! grep -q '/proc:' "$log" || ! grep -q '/dev:' "$log" ||
        ! grep -q 'fork: parent_pid=' "$trace" ||
        grep -Eq 'EXCEPTION:|panicked|exited with status|errno' "$log" "$trace"; then
        echo "$mode userspace smoke FAILED (status $status); see $log" >&2
        exit 1
    fi
    echo "$mode userspace smoke passed"
done

[[ "$(sha256sum "$disk")" == "$before" ]] || {
    echo "Source VHD changed!" >&2
    exit 1
}
echo "Source VHD unchanged"
