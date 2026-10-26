#!/usr/bin/env bash
# Host-only stand-in: prove the protection harness requires faults and task status.
set -euo pipefail
[[ $# == 8 && $1 == run && $3 == --headless && $4 == --vhd &&
   $6 == -- && $7 == -snapshot && $8 == -no-reboot ]]
trace="$TMPDIR/krust-trace-$$.log"
: > "$trace"
printf 'headless kernel trace log: %s\n' "$trace"
while :; do
    printf 'krust> '
    IFS= read -r command
    printf '%s\r\n' "$command"
    case "$command" in
        "cd /") ;;
        "/bin/krust-mmap-protection ok") printf 'KRUST_PROTECTION_OK\r\n' ;;
        "/bin/busybox echo KRUST_PARENT_OK")
            printf 'KRUST_PARENT_OK\r\n'
            if [[ "$MMAP_TEST_CASE" == unexpected-fault ]]; then
                printf 'ERROR: EXCEPTION: PAGE FAULT\n' >> "$trace"
            fi ;;
        "/bin/krust-mmap-protection "*)
            probe=${command##* }
            printf 'KRUST_PROTECTION_ARMED %s\r\n' "$probe"
            if [[ "$MMAP_TEST_CASE" != missing-status ]]; then
                printf '/bin/krust-mmap-protection: exited with status 139\r\n'
            fi
            case "$probe" in
                ro-write|exec-write|rx-write) code='PROTECTION_VIOLATION | CAUSED_BY_WRITE | USER_MODE' ;;
                nx-*) code='PROTECTION_VIOLATION | USER_MODE | INSTRUCTION_FETCH' ;;
                none-read) code='USER_MODE' ;;
                none-write) code='CAUSED_BY_WRITE | USER_MODE' ;;
                none-exec) code='USER_MODE | INSTRUCTION_FETCH' ;;
            esac
            [[ "$MMAP_TEST_CASE" != wrong-fault ]] || code='CAUSED_BY_WRITE'
            if [[ "$MMAP_TEST_CASE" != missing-fault ]]; then
                printf 'ERROR: EXCEPTION: PAGE FAULT\nERROR: Error Code: PageFaultErrorCode(%s)\nERROR: page fault in user task pid=4 - terminating task\n' "$code" >> "$trace"
            fi ;;
        exit)
            if [[ "$MMAP_TEST_CASE" == shutdown-fault ]]; then
                printf 'ERROR: EXCEPTION: PAGE FAULT\n' >> "$trace"
            fi
            exit 0 ;;
        *) exit 8 ;;
    esac
done
