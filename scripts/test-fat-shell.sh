#!/usr/bin/env bash
# Exercise FAT shell subprocesses and recursive interpreter failures in QEMU.
set -euo pipefail
cd "$(dirname "$0")/.."
if (( $# > 2 )); then
    echo "Usage: $0 [busybox [runner]]" >&2
    exit 2
fi
binary=$(realpath -e "${1:-target/rootfs/bin/busybox}")
runner=$(realpath -e "${2:-target/debug/krust}")
for tool in cc setsid timeout sha256sum; do
    command -v "$tool" >/dev/null || { echo "Missing required tool: $tool" >&2; exit 2; }
done
work="$PWD/target/fat-shell-$$"
mkdir "$work"
active=
input=
output=
stop_guest() {
    [[ -z "$input" ]] || { exec {input}>&-; input=; }
    [[ -z "$output" ]] || { exec {output}<&-; output=; }
    if [[ -n "$active" ]]; then
        kill -TERM -- "-$active" 2>/dev/null || true
        sleep 0.2
        kill -KILL -- "-$active" 2>/dev/null || true
        wait "$active" 2>/dev/null || true
        active=
    fi
}
cleanup() {
    status=$?
    trap - EXIT
    stop_guest
    for mode in bios uefi; do
        log="$PWD/target/fat-shell-$mode.log"
        if [[ -f "$log" ]]; then
            trace=$(sed -n 's/^headless kernel trace log: //p' "$log" | tr -d '\r')
            if [[ "$trace" == "$work"/krust-trace-*.log && -f "$trace" ]]; then
                cp "$trace" "target/fat-shell-$mode-kernel.log"
            fi
        fi
    done
    [[ "$status" == 0 ]] || echo "FAT shell test failed; see target/fat-shell-*-{,kernel.}log" >&2
    rm -rf -- "$work"
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap '' PIPE
# Switch formats in-place to verify no stale symlinks or recursive sh scripts.
bash scripts/install-busybox-applets.sh "$binary" "$work/root" ext4
[[ -L "$work/root/bin/sh" ]]
bash scripts/install-busybox-applets.sh "$binary" "$work/root" fat
for shell in sh ash; do
    [[ ! -L "$work/root/bin/$shell" ]]
    cmp "$binary" "$work/root/bin/$shell"
    [[ "$("$work/root/bin/$shell" -c 'echo HOST_SHELL_OK')" == HOST_SHELL_OK ]]
done
[[ ! -e "$work/root/bin/bash" ]]
[[ "$(head -1 "$work/root/bin/ls")" == '#!/bin/busybox sh' ]]
printf '#!/bin/selfloop\n' > "$work/root/bin/selfloop"
printf '#!/bin/cycleb\n' > "$work/root/bin/cyclea"
printf '#!/bin/cyclea\n' > "$work/root/bin/cycleb"
for index in 1 2 3 4; do
    printf '#!/bin/chain%s\n' "$((index + 1))" > "$work/root/bin/chain$index"
done
printf '#!/bin/busybox echo\n' > "$work/root/bin/chain5"
printf '#!/bin/busybox basename\n' > "$work/root/bin/direct"
printf '#!/bin/nested2\n' > "$work/root/bin/nested1"
printf '#!/bin/nested3\n' > "$work/root/bin/nested2"
printf '#!/bin/nested4\n' > "$work/root/bin/nested3"
printf '#!/bin/busybox echo\n' > "$work/root/bin/nested4"
cc -std=c11 -O2 -Wall -Wextra -Werror -ffreestanding -nostdlib -static -no-pie \
    -fno-pie -fno-stack-protector -mno-red-zone -Wl,--build-id=none \
    scripts/tests/process-guest.c -o "$work/root/bin/proctest"
"$runner" mkdisk "$work/root" "$work/disk.vhd" --format fat > "$work/mkdisk.log" 2>&1
printf 'syscall_trace = true\n' >> "$work/disk.vhd.boot.toml"
before=$(sha256sum "$work/disk.vhd")
commands=(
    "/bin/sh -c pwd"
    "/bin/ash -c pwd"
    "/bin/proctest"
    '/bin/busybox sh -c ls;ls;echo${IFS}KRUST_ASH_SUBPROCESS_OK'
    "/bin/direct"
    "/bin/nested1"
    "/bin/selfloop"
    "echo KRUST_AFTER_SELF_LOOP"
    "/bin/cyclea"
    "echo KRUST_AFTER_CYCLE"
    "/bin/chain1"
    "echo KRUST_AFTER_LIMIT"
    "/bin/sh -c pwd"
    "/bin/busybox sh"
    "exit"
)
for mode in bios uefi; do
    log="$PWD/target/fat-shell-$mode.log"
    : > "$log"
    coproc GUEST {
        exec setsid timeout --signal=TERM --kill-after=2s 120s \
            env TMPDIR="$work" "$runner" run "$mode" --headless --vhd "$work/disk.vhd" -- \
            -snapshot -no-reboot 2>&1
    }
    active=$GUEST_PID
    exec {input}>&"${GUEST[1]}"
    exec {output}<&"${GUEST[0]}"
    next=0
    interactive_next=0
    interactive_commands=("ls" "ls" "echo KRUST_INTERACTIVE_OK" "exit")
    suffix=
    response=
    end=$((SECONDS + 120))
    while (( SECONDS < end )); do
        if ! IFS= read -r -t "$((end - SECONDS))" -N 1 char <&"$output"; then
            break
        fi
        printf '%s' "$char" >> "$log"
        response+="$char"
        suffix="${suffix}${char}"
        if (( ${#suffix} > 8 )); then
            suffix="${suffix: -8}"
        fi
        if (( next > 0 && interactive_next < ${#interactive_commands[@]} )) &&
            [[ "${commands[$((next - 1))]}" == "/bin/busybox sh" && "$suffix" == *"# " ]]; then
            printf '%s\n' "${interactive_commands[$interactive_next]}" >&"$input"
            interactive_next=$((interactive_next + 1))
            suffix=
        fi
        if [[ "$suffix" == *"krust> " ]]; then
            if (( next > 0 )); then
                previous=${commands[$((next - 1))]}
                clean=$(printf '%s' "$response" | tr -d '\r' | sed -E $'s/\033\\[[0-9;]*m//g; s/krust> $//' |
                    awk -v command="$previous" 'NR == 1 && $0 == command { next } { print }')
                case "$previous" in
                    /bin/selfloop|/bin/cyclea|/bin/chain1)
                        [[ "$clean" == "$previous: exited with status 127" ]] ;;
                    "echo "*) [[ "$clean" == "${previous#echo }" ]] ;;
                    /bin/direct) [[ "$clean" == direct ]] ;;
                    /bin/nested1) [[ "$clean" == '/bin/nested4 /bin/nested3 /bin/nested2 /bin/nested1' ]] ;;
                    "/bin/sh -c pwd"|"/bin/ash -c pwd") [[ "$clean" == / ]] ;;
                    /bin/proctest) [[ "$clean" == KRUST_PROCESS_OK ]] ;;
                    "/bin/busybox sh")
                        [[ "$interactive_next" == 4 ]]
                        grep -q KRUST_INTERACTIVE_OK <<< "$clean"
                        [[ "$(grep -cw bin <<< "$clean")" == 2 ]]
                        ! grep -Eq 'not found|Usage:|errno|status' <<< "$clean" ;;
                    '/bin/busybox sh -c ls;ls;echo${IFS}KRUST_ASH_SUBPROCESS_OK')
                        grep -qx KRUST_ASH_SUBPROCESS_OK <<< "$clean"
                        [[ "$(grep -cw bin <<< "$clean")" == 2 ]]
                        ! grep -Eq 'not found|Usage:|errno|status' <<< "$clean" ;;
                    *) [[ -z "$clean" ]] ;;
                esac
            fi
            (( next < ${#commands[@]} )) || { echo "unexpected extra prompt" >&2; exit 1; }
            printf '%s\n' "${commands[$next]}" >&"$input"
            next=$((next + 1))
            suffix=
            response=
        fi
    done
    [[ "$next" == "${#commands[@]}" ]]
    while kill -0 "$active" 2>/dev/null; do
        (( SECONDS < end )) || { echo "$mode FAT shell timed out" >&2; exit 1; }
        sleep 0.05
    done
    wait "$active"
    active=
    stop_guest
    trace=$(sed -n 's/^headless kernel trace log: //p' "$log" | tr -d '\r')
    [[ "$trace" == "$work"/krust-trace-*.log && -f "$trace" ]]
    [[ "$(grep -c 'interpreter redirect limit exceeded' "$trace")" == 3 ]]
    [[ "$(grep -c 'errno -40' "$trace")" == 3 ]]
    ! grep -q 'Kernel panic' "$trace"
    [[ "$(grep -c 'EXCEPTION:' "$trace")" == 1 ]]
    [[ "$(grep -c 'EXCEPTION: PAGE FAULT' "$trace")" == 1 ]]
    [[ "$(grep -c 'Error Code:.*USER_MODE' "$trace")" == 1 ]]
    [[ "$(grep -c 'page fault in user task' "$trace")" == 1 ]]
    cp "$trace" "target/fat-shell-$mode-kernel.log"
    [[ "$(sha256sum "$work/disk.vhd")" == "$before" ]]
    echo "$mode FAT shell and bounded interpreter redirects passed"
done
