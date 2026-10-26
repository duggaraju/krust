#!/usr/bin/env bash
# Controlled host-only stand-in for the runner and its QEMU child.
set -euo pipefail
[[ $# == 8 && $1 == run && $3 == --headless && $4 == --vhd &&
   $6 == -- && $7 == -snapshot && $8 == -no-reboot ]]
[[ "$TMPDIR" == "$PWD"/target/userspace-smoke-* && -d "$TMPDIR" ]]
printf '%s\n' "$TMPDIR" >> "$SMOKE_TEST_STATE/workdirs"
printf '%s\n' "$$" >> "$SMOKE_TEST_STATE/pids"
touch "$TMPDIR/generated.img" "$TMPDIR/generated.ramdisk"
trace="$TMPDIR/krust-trace-$$.log"
printf 'fork: parent_pid=1\n' > "$trace"
case "$SMOKE_TEST_CASE" in
    trace-success)
        printf 'INFO : syscall entry: pid=4 nr=9 (mmap) args=[0x0]\r\nINFO : syscall exit: pid=4 nr=9 (mmap) ret=-12\r\nERROR: syscall failed: nr=9 (mmap) errno=-12\r\n' >> "$trace" ;;
    kernel-errno) printf 'ERROR: unexpected errno=-12\n' >> "$trace" ;;
    kernel-panic) printf 'panicked at kernel probe\n' >> "$trace" ;;
esac
if [[ "$SMOKE_TEST_CASE" == probe-success || "$SMOKE_TEST_CASE" == probe-missing-marker ]]; then
    printf 'INFO : virtual_address_offset: 0x10000000000\nINFO : Handling Segment: Ph64(ProgramHeader64 { type_: Ok(Load), virtual_addr: 1234, physical_addr: 0 })\n' >> "$trace"
fi
printf 'headless kernel trace log: %s\n' "$trace"

# The runner must reap its QEMU stand-in when the harness kills their group.
sleep 300 &
child=$!
printf '%s\n' "$child" >> "$SMOKE_TEST_STATE/pids"
cleanup() {
    kill -TERM "$child" 2>/dev/null || true
    wait "$child" 2>/dev/null || true
}
trap cleanup EXIT
trap 'exit 143' TERM

case "$SMOKE_TEST_CASE" in
    missing-shell) printf 'booted without a shell\n'; wait "$child"; exit 1 ;;
    mutate-failure) printf 'changed\n' >> "$5"; exit 7 ;;
    runner-failure) exit 7 ;;
    closed-output) exec 1>&- 2>&-; wait "$child"; exit 1 ;;
esac

expected=(
    "cd /"
)
if [[ "$SMOKE_TEST_CASE" == probe-* ]]; then
    expected+=("/bin/krust-mmap-probe 0x10000001000")
fi
expected+=(
    "/bin/busybox pwd"
    "/bin/busybox echo KRUST_ECHO_OK"
    "/bin/busybox ls /proc /dev"
    "/bin/busybox cat /proc/meminfo"
    "/bin/busybox cat /proc/cpuinfo"
    "/bin/busybox true"
    "/bin/busybox echo KRUST_FORK_EXEC_OK"
    "exit"
)
for index in "${!expected[@]}"; do
    # Split a prompt across writes and ensure the host has not sent commands
    # early (especially not a whole batch after the initial boot prompt).
    if IFS= read -r -t 0.03 early; then
        echo "Command arrived before its prompt: $early" >&2
        exit 8
    fi
    if [[ "$SMOKE_TEST_CASE" == timeout && $index == 4 ]]; then
        printf 'krust>'
        wait "$child"
        exit 1
    fi
    printf 'kru'
    sleep 0.01
    printf 'st> '
    IFS= read -r command
    [[ "$command" == "${expected[$index]}" ]] || exit 9
    printf '%s: %s\n' "$2" "$command" >> "$SMOKE_TEST_STATE/commands"
    # Serial echo is allowed, but must not satisfy output assertions.
    printf '%s\r\n' "$command"
    response_index=$index
    if [[ "$SMOKE_TEST_CASE" == probe-* && $index -ge 1 ]]; then
        if (( index == 1 )); then
            [[ "$SMOKE_TEST_CASE" == probe-missing-marker ]] || printf 'KRUST_MMAP_KERNEL_OK\r\n'
            printf 'KRUST_MMAP_OK\r\n'
            continue
        fi
        response_index=$((index - 1))
    fi
    case "$response_index" in
        1) [[ "$SMOKE_TEST_CASE" == bad-pwd ]] || printf '/\r\n' ;;
        2) [[ "$SMOKE_TEST_CASE" == echo-only ]] || printf 'KRUST_ECHO_OK\r\n' ;;
        3) printf '/dev:\r\n\033[1;35mnull\033[m ttyS0\r\n/proc:\r\ncpuinfo \033[1;32mmeminfo\033[m\r\n' ;;
        4) [[ "$SMOKE_TEST_CASE" == bad-cat ]] ||
           printf 'MemTotal:       65536 kB\r\nMemFree:        1234 kB\r\nBuffers:        0 kB\r\n' ;;
        5) printf 'processor\t: 0\r\nvendor_id\t: GenuineIntel\r\n' ;;
        6) if [[ "$SMOKE_TEST_CASE" == command-failure ]]; then
               printf '/bin/busybox: exited with status 1\r\n'
           fi ;;
        7) printf 'KRUST_FORK_EXEC_OK\r\n' ;;
        8) if [[ "$SMOKE_TEST_CASE" == mutate-success ]]; then
               printf 'changed\n' >> "$5"
           fi
           exit 0 ;;
    esac
done
