#!/usr/bin/env bash
# Internal real-guest protection runner; expected faults are scoped per command.
set -euo pipefail
cd "$(dirname "$0")/.."
if (( $# != 2 )); then
    echo "Usage: $0 <fixture-ext4-disk.vhd> <runner> (normally invoked by test-mmap-guest.sh)" >&2
    exit 2
fi
disk=$(realpath -e "$1")
runner=$(realpath -e "$2")
work="$PWD/target/mmap-protections-$$"
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
    stop_guest
    rm -rf -- "$work"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap '' PIPE

errors='EXCEPTION:|panicked|exited with status|errno|command not found|KRUST_PROTECTION_FAIL'
clean_kernel() {
    sed -E '/^ERROR: syscall failed: nr=[0-9]+ \([a-z0-9_]+\) errno=-[0-9]+\r?$/d' "$1"
}
check_response() {
    local command=$1 probe=${1##* } expected=
    tr -d '\r' < "$work/response" | sed -E $'s/\033\\[[0-9;]*m//g' > "$work/clean"
    tail -c "+$((trace_offset + 1))" "$trace" | tr -d '\r' > "$work/kernel-response"
    if [[ "$command" == "/bin/krust-mmap-protection "* && "$probe" != ok ]]; then
        grep -qx "KRUST_PROTECTION_ARMED $probe" "$work/clean" || return 1
        grep -qx '/bin/krust-mmap-protection: exited with status 139' "$work/clean" || return 1
        ! sed '\|^/bin/krust-mmap-protection: exited with status 139$|d' "$work/clean" |
            grep -Eq "$errors" || return 1
        [[ $(grep -c 'EXCEPTION: PAGE FAULT' "$work/kernel-response") == 1 ]] || return 1
        [[ $(grep -c 'page fault in user task pid=.* - terminating task' "$work/kernel-response") == 1 ]] || return 1
        case "$probe" in
            ro-write|exec-write|rx-write) expected='PROTECTION_VIOLATION \| CAUSED_BY_WRITE \| USER_MODE' ;;
            nx-*) expected='PROTECTION_VIOLATION \| USER_MODE \| INSTRUCTION_FETCH' ;;
            none-read) expected='USER_MODE' ;;
            none-write) expected='CAUSED_BY_WRITE \| USER_MODE' ;;
            none-exec) expected='USER_MODE \| INSTRUCTION_FETCH' ;;
        esac
        grep -Eq "^ERROR: Error Code: PageFaultErrorCode\\($expected\\)$" "$work/kernel-response" || return 1
        clean_kernel "$work/kernel-response" | sed '/^ERROR: EXCEPTION: PAGE FAULT$/d' |
            grep -Eq "$errors" && return 1
        return 0
    fi
    ! grep -Eq "$errors" "$work/clean" || return 1
    ! clean_kernel "$work/kernel-response" | grep -Eq "$errors" || return 1
    case "$command" in
        "/bin/krust-mmap-protection ok") grep -qx 'KRUST_PROTECTION_OK' "$work/clean" ;;
        "/bin/busybox echo KRUST_PARENT_OK") grep -qx 'KRUST_PARENT_OK' "$work/clean" ;;
    esac
}

commands=("cd /" "/bin/krust-mmap-protection ok")
for probe in ro-write exec-write rx-write nx-read nx-write nx-rw none-read none-write none-exec; do
    commands+=("/bin/krust-mmap-protection $probe" "/bin/busybox echo KRUST_PARENT_OK")
done
commands+=("exit")
for mode in bios uefi; do
    log="$PWD/target/mmap-protections-$mode.log"
    : > "$log"
    rm -f "target/mmap-protections-$mode-response.log" "target/mmap-protections-$mode-kernel.log"
    : > "$work/response"
    coproc GUEST {
        exec setsid timeout --signal=TERM --kill-after=2s 120s \
            env TMPDIR="$work" "$runner" run "$mode" --headless --vhd "$disk" -- \
            -snapshot -no-reboot 2>&1
    }
    active=$GUEST_PID
    exec {input}>&"${GUEST[1]}"
    exec {output}<&"${GUEST[0]}"
    next=0
    suffix=
    trace=
    trace_offset=0
    end=$((SECONDS + 120))
    failure=
    while :; do
        remaining=$((end - SECONDS))
        if (( remaining <= 0 )); then failure=timeout; break; fi
        if ! IFS= read -r -t "$remaining" -N 1 char <&"$output"; then break; fi
        printf '%s' "$char" >> "$log"
        printf '%s' "$char" >> "$work/response"
        suffix="${suffix}${char}"
        if (( ${#suffix} > 8 )); then suffix="${suffix: -8}"; fi
        if [[ "$suffix" == *"krust> " && $next -lt ${#commands[@]} ]]; then
            if [[ -z "$trace" ]]; then
                trace=$(sed -n 's/^headless kernel trace log: //p' "$log" | tr -d '\r')
                if [[ "$trace" != "$work"/krust-trace-*.log || ! -f "$trace" ]]; then
                    failure="missing kernel trace"; break
                fi
            fi
            if (( next > 0 )); then
                if ! check_response "${commands[$((next - 1))]}"; then
                    failure="command failed: ${commands[$((next - 1))]}"; break
                fi
            elif clean_kernel "$trace" | grep -Eq "$errors"; then
                failure="boot fault"; break
            fi
            trace_offset=$(wc -c < "$trace")
            if ! printf '%s\n' "${commands[$next]}" >&"$input"; then
                failure="closed input"; break
            fi
            next=$((next + 1))
            suffix=
            : > "$work/response"
        fi
    done
    if [[ -z "$failure" ]]; then
        while kill -0 "$active" 2>/dev/null; do
            if (( SECONDS >= end )); then failure=timeout; break; fi
            sleep 0.05
        done
    fi
    status=0
    if [[ -z "$failure" ]]; then wait "$active" || status=$?; fi
    if [[ -z "$failure" && $next == ${#commands[@]} ]] &&
        ! check_response "exit"; then
        failure="shutdown fault"
    fi
    stop_guest
    if [[ -n "$trace" && -f "$trace" ]]; then
        cp "$trace" "target/mmap-protections-$mode-kernel.log"
    fi
    if [[ -n "$failure" || $status != 0 || $next != ${#commands[@]} ]]; then
        cp "$work/response" "target/mmap-protections-$mode-response.log"
        echo "$mode mmap protections FAILED (${failure:-incomplete suite}; status $status); see $log" >&2
        exit 1
    fi
    echo "$mode mmap protections passed (all combinations, 9 verified user faults, parent healthy)"
done
