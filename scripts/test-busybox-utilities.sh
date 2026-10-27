#!/usr/bin/env bash
# Exercise installed applet paths through BusyBox ash, never kernel built-ins.
set -euo pipefail
export LC_ALL=C
cd "$(dirname "$0")/.."
if (( $# > 3 )); then
    echo "Usage: $0 [busybox [runner [fat|ext4]]]" >&2
    exit 2
fi
binary=$(realpath -e "${1:-target/rootfs/bin/busybox}")
runner=$(realpath -e "${2:-target/debug/krust}")
format=${3:-fat}
[[ "$format" == fat || "$format" == ext4 ]] || { echo "format must be fat or ext4" >&2; exit 2; }
for tool in setsid timeout sha256sum gzip tar; do
    command -v "$tool" >/dev/null || { echo "Missing required tool: $tool" >&2; exit 2; }
done
source scripts/tests/utility-cases.sh
work="$PWD/target/busybox-utilities-$$"
mkdir "$work"
active=
input=
output=
trace=
before=
report="$PWD/target/busybox-utilities-$format.tsv"
printf 'firmware\tutility\tresult\texpected_status\tactual_status\tlaunched\n' > "$report"
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
    if [[ "$trace" == "$work"/krust-trace-*.log && -f "$trace" ]]; then
        cp "$trace" "target/busybox-utilities-$format-$mode-incomplete-kernel.log"
    fi
    if [[ -n "$before" && "$(sha256sum "$work/disk.vhd")" != "$before" ]]; then
        echo "Source VHD changed!" >&2
        status=1
    fi
    rm -rf -- "$work"
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap '' PIPE
bash scripts/install-busybox-applets.sh "$binary" "$work/root" "$format"
mkdir -p "$work/root/fixture/empty" "$work/root/cases" "$work/root/etc"
printf 'alpha\nbeta\nalpha\n' > "$work/root/fixture/input"
printf 'alpha\nbeta\nalpha\n' > "$work/root/fixture/zipin"
printf 'alpha\nalpha\nbeta\n' > "$work/root/fixture/repeat"
printf 'one two\n' > "$work/root/fixture/one"
printf AB > "$work/root/fixture/bytes"
printf '\000UTILITY_STRING\000' > "$work/root/fixture/strings"
printf 'move-data\n' > "$work/root/fixture/move"
printf 'remove-data\n' > "$work/root/fixture/remove"
printf 'mode-data\n' > "$work/root/fixture/mode"
printf 'owner-data\n' > "$work/root/fixture/owner"
gzip -n -c "$work/root/fixture/input" > "$work/root/fixture/data.gz"
tar -cf "$work/root/fixture/data.tar" -C "$work/root/fixture" input
cat > "$work/root/etc/fstab" <<EOF
/dev/sda / $format defaults 0 0
proc /proc proc defaults 0 0
dev /dev dev defaults 0 0
none /scratch ramfs defaults 0 0
EOF
utilities=()
for path in "$work/root/bin/"*; do
    name=${path##*/}
    [[ "$name" != busybox ]] || continue
    utility_case "$name"
    utilities+=("$name")
    printf '%s\nutility_status=$?\necho\necho KRUST_UTILITY_DONE:%s:$utility_status\n' \
        "$utility_command" "$name" > "$work/root/cases/$name"
done
(( ${#utilities[@]} > 0 )) || { echo "No installed utilities found" >&2; exit 1; }
"$runner" mkdisk "$work/root" "$work/disk.vhd" --format "$format" > "$work/mkdisk.log" 2>&1
printf 'syscall_trace = true\n' >> "$work/disk.vhd.boot.toml"
before=$(sha256sum "$work/disk.vhd")
failures=0
for mode in bios uefi; do
    log="$PWD/target/busybox-utilities-$format-$mode.log"
    kernel_log="$PWD/target/busybox-utilities-$format-$mode-kernel.log"
    : > "$log"
    : > "$kernel_log"
    trace=
    for name in "${utilities[@]}"; do
        utility_case "$name"
        case_log="target/busybox-utilities-$format-$mode-$name-console.log"
        : > "$case_log"
        if [[ -z "$active" ]]; then
            trace=
            trace_start=0
            coproc GUEST {
                exec setsid timeout --signal=TERM --kill-after=2s 900s \
                    env TMPDIR="$work" "$runner" run "$mode" --headless --vhd "$work/disk.vhd" -- \
                    -snapshot -no-reboot 2>&1
            }
            active=$GUEST_PID
            exec {input}>&"${GUEST[1]}"
            exec {output}<&"${GUEST[0]}"
            next=0
        else
            printf '/bin/busybox sh /cases/%s\n' "$name" >&"$input"
            next=1
        fi
        suffix=
        : > "$work/response"
        end=$((SECONDS + 60))
        while (( SECONDS < end )); do
            if ! IFS= read -r -t "$((end - SECONDS))" -N 1 char <&"$output"; then break; fi
            printf '%s' "$char" >> "$case_log"
            if (( next < 2 )); then printf '%s' "$char" >> "$work/response"; fi
            suffix+="$char"
            if (( ${#suffix} > 8 )); then suffix="${suffix: -8}"; fi
            if [[ "$suffix" != *"krust> " ]]; then continue; fi
            if (( next == 0 )); then
                trace=$(sed -n 's/^headless kernel trace log: //p' "$case_log" | tr -d '\r')
                [[ "$trace" == "$work"/krust-trace-*.log && -f "$trace" ]]
                printf '/bin/busybox sh /cases/%s\n' "$name" >&"$input"
                : > "$work/response"
            elif (( next == 1 )); then
                next=2
                break
            else
                echo "$mode $name: unexpected extra prompt" >&2
                break
            fi
            next=$((next + 1))
            suffix=
        done
        result=PASS
        if (( next != 2 )); then
            echo "$mode $name: incomplete command or timeout; continuing with a fresh guest" >&2
            result=FAIL
            stop_guest
        fi
        cat "$case_log" >> "$log"
        response_log="target/busybox-utilities-$format-$mode-$name.log"
        tr -d '\r' < "$work/response" |
            sed -E $'s/\033\\[[0-9;]*m//g; s/krust> $//' > "$response_log"
        marker=$(sed -nE "s/^(KRUST_UTILITY_DONE:$name:[0-9]+)$/\\1/p" "$response_log")
        actual=${marker##*:}
        response=$(sed "/^KRUST_UTILITY_DONE:$name:[0-9]*$/d" "$response_log")
        if (( next != 2 )) || [[ "$marker" != "KRUST_UTILITY_DONE:$name:"* ]] ||
            ! utility_validate "$response" "$actual"; then
            result=FAIL
        fi
        launched=no
        if [[ -z "$trace" ]]; then
            echo "$mode $name: no kernel trace found" >&2
            result=FAIL
        else
            trace_end=$(wc -l < "$trace")
            case_trace="target/busybox-utilities-$format-$mode-$name-kernel.log"
            sed -n "$((trace_start + 1)),${trace_end}p" "$trace" > "$case_trace"
            trace_start=$trace_end
            if grep -Eq "execve: (loading|redirecting|symlink resolved) '/bin/$name'" "$case_trace"; then
                launched=yes
            else
                echo "$mode $name: installed utility path was not executed" >&2
                result=FAIL
            fi
            if grep -Eq 'Kernel panic|EXCEPTION:' "$case_trace"; then
                echo "$mode $name: kernel panic or exception detected" >&2
                result=FAIL
            fi
            cat "$case_trace" >> "$kernel_log"
        fi
        [[ "$result" == PASS ]] || failures=$((failures + 1))
        printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$mode" "$name" "$result" "$utility_status" "${actual:-missing}" "$launched" >> "$report"
        echo "$mode $format $name: $result"
    done
    if [[ -n "$active" ]]; then
        printf 'exit\n' >&"$input"
        end=$((SECONDS + 10))
        while kill -0 "$active" 2>/dev/null && (( SECONDS < end )); do sleep 0.05; done
        if kill -0 "$active" 2>/dev/null; then
            echo "$mode: shutdown timeout" >&2
            failures=$((failures + 1))
        elif ! wait "$active"; then
            echo "$mode: guest exited unsuccessfully" >&2
            failures=$((failures + 1))
            active=
        else
            active=
        fi
        stop_guest
    fi
    trace=
    sed -nE 's/.*syscall entry:.*nr=([0-9]+) \(([^)]+)\).*/\1 \2/p' "$kernel_log" |
        sort -u > "target/busybox-utilities-$format-$mode-syscalls.log"
    sed -n '/unimplemented syscall:/p' "$kernel_log" | sort -u \
        > "target/busybox-utilities-$format-$mode-unimplemented.log"
done
echo "Tested ${#utilities[@]} utilities per firmware; $failures failures; report: $report"
(( failures == 0 ))
