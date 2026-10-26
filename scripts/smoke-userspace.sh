#!/usr/bin/env bash
# Exercise the real ring-3/IRQ entry path against an existing BusyBox ext4 VHD.
set -euo pipefail
cd "$(dirname "$0")/.."

if (( $# > 2 )); then
    echo "Usage: $0 [disk.vhd [runner]]" >&2
    exit 2
fi
deadline=${KRUST_SMOKE_TIMEOUT_SECONDS-120}
shell_probe=${KRUST_SMOKE_SHELL_PROBE-0}
if [[ "$shell_probe" != 0 && "$shell_probe" != 1 ]]; then
    echo "KRUST_SMOKE_SHELL_PROBE must be 0 or 1" >&2
    exit 2
fi
if [[ "${KRUST_SMOKE_MMAP_PROBE-0}" != 0 && "${KRUST_SMOKE_MMAP_PROBE-0}" != 1 ]]; then
    echo "KRUST_SMOKE_MMAP_PROBE must be 0 or 1" >&2
    exit 2
fi
if [[ "${KRUST_SMOKE_CLOEXEC_PROBE-0}" != 0 && "${KRUST_SMOKE_CLOEXEC_PROBE-0}" != 1 ]]; then
    echo "KRUST_SMOKE_CLOEXEC_PROBE must be 0 or 1" >&2
    exit 2
fi
if [[ ! "$deadline" =~ ^([1-9]|[1-9][0-9]|1[01][0-9]|120)$ ]]; then
    echo "KRUST_SMOKE_TIMEOUT_SECONDS must be an integer from 1 to 120" >&2
    exit 2
fi
for tool in realpath sha256sum setsid timeout; do
    command -v "$tool" >/dev/null || { echo "Missing required tool: $tool" >&2; exit 2; }
done
disk=$(realpath -e "${1:-target/krust-disk-ext4.vhd}")
runner=$(realpath -e "${2:-target/debug/krust}")
[[ -f "$disk" && -r "$disk" ]] || { echo "VHD must be a readable file" >&2; exit 2; }
[[ -f "$runner" && -x "$runner" ]] || { echo "Runner must be executable" >&2; exit 2; }
mkdir -p target
work="$PWD/target/userspace-smoke-$$"
# An atomic mkdir must succeed before we own (and may remove) this directory.
mkdir "$work"
active=
input=
output=
before=
stop_guest() {
    [[ -z "$input" ]] || { exec {input}>&-; input=; }
    [[ -z "$output" ]] || { exec {output}<&-; output=; }
    if [[ -n "$active" ]]; then
        # setsid isolates only this runner and its descendants, never other QEMUs.
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
    if [[ -n "$before" ]]; then
        if after=$(sha256sum "$disk") && [[ "$after" == "$before" ]]; then
            echo "Source VHD unchanged"
        else
            echo "Source VHD changed!" >&2
            status=1
        fi
    fi
    rm -rf -- "$work"
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap '' PIPE
before=$(sha256sum "$disk")

commands=(
    "cd /"
)
if [[ "${KRUST_SMOKE_MMAP_PROBE-0}" == 1 ]]; then
    commands+=("/bin/krust-mmap-probe")
fi
if [[ "${KRUST_SMOKE_CLOEXEC_PROBE-0}" == 1 ]]; then
    commands+=("cloexec-test")
fi
commands+=(
    "/bin/busybox pwd"
    "/bin/busybox echo KRUST_ECHO_OK"
    "/bin/busybox ls /proc /dev"
    "/bin/busybox cat /proc/meminfo"
    "/bin/busybox cat /proc/cpuinfo"
    "/bin/busybox true"
    "/bin/busybox echo KRUST_FORK_EXEC_OK"
    "exit"
)
suite=userspace
log_prefix=userspace-smoke
if [[ "$shell_probe" == 1 ]]; then
    if [[ "${KRUST_SMOKE_MMAP_PROBE-0}" != 0 || "${KRUST_SMOKE_CLOEXEC_PROBE-0}" != 0 ]]; then
        echo "Run the shell suite separately from mmap/CLOEXEC probes" >&2
        exit 2
    fi
    source scripts/tests/shell-suite.sh
    shell_suite disk
    commands=("${shell_commands[@]}")
    suite=shell
    log_prefix=shell-smoke
fi
errors='EXCEPTION:|panicked|exited with status|errno|command not found'
kernel_errors() {
    # Negative syscall results include normal userspace probes and retries.
    # Command responses are still checked above, including their errno output.
    sed -E '/^ERROR: syscall failed: nr=[0-9]+ \([a-z0-9_]+\) errno=-[0-9]+\r?$/d' "$1" |
        grep -E "$errors" > /dev/null
}
mmap_probe_address() {
    local trace bias segment address
    trace=$(sed -n 's/^headless kernel trace log: //p' "$1" | tr -d '\r')
    [[ "$trace" == "$work"/krust-trace-*.log && -f "$trace" ]] || return 1
    bias=$(sed -nE '/virtual_address_offset: 0x/{s/.*virtual_address_offset: 0x([0-9a-fA-F]+).*/\1/p;q;}' "$trace")
    segment=$(sed -nE '/Handling Segment:.*type_: Ok\(Load\)/{s/.*virtual_addr: ([0-9a-fA-F]+),.*/\1/p;q;}' "$trace")
    [[ "$bias" =~ ^[0-9a-fA-F]{1,12}$ && "$segment" =~ ^[0-9a-fA-F]{1,12}$ ]] || return 1
    # The bootloader logs the relocation bias and original PT_LOAD vaddr.
    address=$(( (16#$bias + 16#$segment) & ~4095 ))
    (( address > 0 && address < (1 << 47) )) || return 1
    printf '0x%x' "$address"
}
check_response() {
    if [[ "$shell_probe" == 1 ]]; then
        shell_check_response "$1"
        return
    fi
    # Check only this command's response, with full-line anchors so an echoed
    # command containing the expected string can never substitute for output.
    local response="$work/response"
    tr -d '\r' < "$work/raw-response" | sed -E $'s/\033\\[[0-9;]*m//g' > "$response"
    ! grep -Eq "$errors" "$response" || return 1
    case "${commands[$1]}" in
        "cloexec-test") grep -qx 'KRUST_CLOEXEC_BEFORE_OK' "$response" &&
           grep -qx 'KRUST_CLOEXEC_EXEC_OK' "$response" &&
           grep -qx 'KRUST_CLOEXEC_FORK_OK' "$response" ;;
        "/bin/krust-mmap-probe "*) grep -qx 'KRUST_MMAP_OK' "$response" &&
           grep -qx 'KRUST_MMAP_KERNEL_OK' "$response" ;;
        "/bin/busybox pwd") grep -qx '/' "$response" ;;
        "/bin/busybox echo KRUST_ECHO_OK") grep -qx 'KRUST_ECHO_OK' "$response" ;;
        "/bin/busybox ls /proc /dev") grep -qx '/proc:' "$response" && grep -qx '/dev:' "$response" &&
           grep -Eq '(^|[[:space:]])meminfo([[:space:]]|$)' "$response" &&
           grep -Eq '(^|[[:space:]])null([[:space:]]|$)' "$response" ;;
        "/bin/busybox cat /proc/meminfo") grep -Eq '^MemTotal: +[1-9][0-9]* kB$' "$response" &&
           grep -Eq '^MemFree: +[0-9]+ kB$' "$response" &&
           grep -qx 'Buffers:        0 kB' "$response" ;;
        "/bin/busybox cat /proc/cpuinfo") grep -Eq $'^processor\t+: 0$' "$response" &&
           grep -Eq $'^vendor_id\t+: .+$' "$response" ;;
        "/bin/busybox echo KRUST_FORK_EXEC_OK") grep -qx 'KRUST_FORK_EXEC_OK' "$response" ;;
    esac
}

boots=(bios uefi)
if [[ "$shell_probe" == 1 ]]; then
    boots=(bios-disk bios-ramfs uefi-disk uefi-ramfs)
fi
for boot in "${boots[@]}"; do
    mode=${boot%%-*}
    disk_args=(--vhd "$disk")
    shell_last_ticks=0
    if [[ "$shell_probe" == 1 ]]; then
        shell_fs=${boot#*-}
        shell_suite "$shell_fs"
        commands=("${shell_commands[@]}")
        [[ "$shell_fs" != ramfs ]] || disk_args=(--no-vhd)
        # Exercise both shutdown spellings across the two firmware modes.
        commands[${#commands[@]}-1]=exit
        [[ "$mode" != bios ]] || commands[${#commands[@]}-1]=shutdown
    fi
    if [[ "${KRUST_SMOKE_MMAP_PROBE-0}" == 1 ]]; then
        commands[1]="/bin/krust-mmap-probe"
    fi
    log="$PWD/target/$log_prefix-$boot.log"
    : > "$log"
    rm -f "target/$log_prefix-$boot-kernel.log" "target/$log_prefix-$boot-response.log"
    : > "$work/raw-response"
    coproc GUEST {
        exec setsid timeout --signal=TERM --kill-after=2s "${deadline}s" \
            env TMPDIR="$work" "$runner" run "$mode" --headless "${disk_args[@]}" -- \
            -snapshot -no-reboot 2>&1
    }
    active=$GUEST_PID
    exec {input}>&"${GUEST[1]}"
    exec {output}<&"${GUEST[0]}"
    next=0
    suffix=
    failure=
    end=$((SECONDS + deadline))
    while :; do
        remaining=$((end - SECONDS))
        if (( remaining <= 0 )); then
            failure="timeout after ${deadline}s"
            break
        fi
        if IFS= read -r -t "$remaining" -N 1 char <&"$output"; then
            :
        else
            read_status=$?
            if (( read_status > 128 )); then
                failure="timeout after ${deadline}s"
            fi
            break
        fi
        printf '%s' "$char" >> "$log"
        printf '%s' "$char" >> "$work/raw-response"
        suffix="${suffix}${char}"
        if [[ ${#suffix} -gt 8 ]]; then
            suffix="${suffix: -8}"
        fi
        if [[ "$suffix" == *"krust> " && $next -lt ${#commands[@]} ]]; then
            if (( next == 0 )) && grep -Eq "$errors" "$work/raw-response"; then
                failure="boot diagnostic before first command"
                break
            fi
            # A complete prompt ends the previous command's response.
            if (( next > 0 )) && ! check_response "$((next - 1))"; then
                cp "$work/response" "target/$log_prefix-$boot-response.log"
                failure="command failed: ${commands[$((next - 1))]}"
                break
            fi
            if [[ "${commands[$next]}" == "/bin/krust-mmap-probe" ]]; then
                if ! address=$(mmap_probe_address "$log"); then
                    failure="cannot derive kernel mapping address from boot trace"
                    break
                fi
                commands[$next]+=" $address"
                printf '%s mmap kernel page from boot PT_LOAD: %s\n' "$mode" "$address"
                printf '\nmmap kernel page from boot PT_LOAD: %s\n' "$address" >> "$log"
            fi
            if ! printf '%s\n' "${commands[$next]}" >&"$input"; then
                failure="runner closed command input"
                break
            fi
            next=$((next + 1))
            suffix=
            : > "$work/raw-response"
        fi
    done
    status=0
    # EOF is not proof the runner exited; keep the same deadline for wait.
    if [[ -z "$failure" ]]; then
        while kill -0 "$active" 2>/dev/null; do
            if (( SECONDS >= end )); then
                failure="timeout after ${deadline}s"
                break
            fi
            sleep 0.05
        done
        if [[ -z "$failure" ]]; then
            wait "$active" || status=$?
        fi
    fi
    stop_guest
    if [[ "$shell_probe" == 1 && -z "$failure" && $next == ${#commands[@]} ]] &&
        ! check_response "$((next - 1))"; then
        cp "$work/response" "target/$log_prefix-$boot-response.log"
        failure="unexpected shutdown output"
    fi
    trace=$(sed -n 's/^headless kernel trace log: //p' "$log" | tr -d '\r')
    valid_trace=false
    if [[ "$trace" == "$work"/krust-trace-*.log && -f "$trace" ]]; then
        cp "$trace" "target/$log_prefix-$boot-kernel.log"
        valid_trace=true
    fi
    if [[ $status == 124 || $status == 137 ]]; then
        failure="timeout after ${deadline}s (status $status)"
    fi
    if [[ -n "$failure" || $status != 0 || $next != ${#commands[@]} || "$valid_trace" != true ]] ||
        { [[ "$boot" != *-ramfs ]] && ! grep -q 'fork: parent_pid=' "$trace"; } ||
        { [[ "$shell_probe" == 0 ]] && grep -Eq "$errors" "$log"; } ||
        kernel_errors "$trace"; then
        echo "$boot $suite smoke FAILED (${failure:-incomplete suite or runner failure}; status $status, commands $next/${#commands[@]}); see $log" >&2
        exit 1
    fi
    echo "$boot $suite smoke passed"
done
