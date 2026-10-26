#!/usr/bin/env bash
# No QEMU, disk tooling, or build needed.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p target
work="$PWD/target/userspace-smoke-tests-$$"
mkdir "$work"
sentinel=
cleanup() {
    if [[ -n "$sentinel" ]]; then
        kill -TERM "$sentinel" 2>/dev/null || true
        wait "$sentinel" 2>/dev/null || true
    fi
    rm -rf -- "$work"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
sleep 300 &
sentinel=$!
runner="$PWD/scripts/tests/smoke-userspace-runner.sh"

for scenario in success trace-success probe-success probe-missing-address probe-missing-marker cloexec-success cloexec-missing-before cloexec-missing-exec cloexec-missing-fork cloexec-echo-only shell-success shell-echo-only shell-env-loss shell-no-clear shell-wrong-diagnostic shell-missing-recovery shell-uptime-backwards shell-bad-content shell-cwd-loss shell-shutdown-error shell-timeout kernel-errno kernel-panic missing-shell command-failure timeout closed-output echo-only bad-pwd bad-cat runner-failure mutate-failure mutate-success; do
    state="$work/$scenario"
    mkdir "$state"
    printf 'original VHD fixture\n' > "$state/disk.vhd"
    before=$(sha256sum "$state/disk.vhd")
    log="$PWD/target/userspace-smoke-test-$scenario.log"
    status=0
    started=$SECONDS
    probe=0
    cloexec=0
    shell=0
    deadline=3
    [[ "$scenario" != probe-* ]] || probe=1
    [[ "$scenario" != cloexec-* ]] || cloexec=1
    if [[ "$scenario" == shell-* ]]; then
        shell=1
        deadline=10
        [[ "$scenario" != shell-timeout ]] || deadline=3
    fi
    SMOKE_TEST_STATE="$state" SMOKE_TEST_CASE="$scenario" KRUST_SMOKE_TIMEOUT_SECONDS="$deadline" \
        KRUST_SMOKE_MMAP_PROBE="$probe" \
        KRUST_SMOKE_CLOEXEC_PROBE="$cloexec" \
        KRUST_SMOKE_SHELL_PROBE="$shell" \
        scripts/smoke-userspace.sh "$state/disk.vhd" "$runner" > "$log" 2>&1 || status=$?
    (( SECONDS - started <= (shell == 1 ? 45 : 8) ))
    if [[ "$scenario" == shell-success ]]; then
        [[ $status == 0 ]]
        for boot in bios-disk bios-ramfs uefi-disk uefi-ramfs; do
            grep -q "^$boot shell smoke passed$" "$log"
        done
        [[ $(wc -l < "$state/commands") == 220 ]]
    elif [[ "$scenario" == shell-* ]]; then
        [[ $status == 1 ]]
        if [[ "$scenario" == shell-bad-content || "$scenario" == shell-cwd-loss ]]; then
            grep -q '^bios-disk shell smoke passed$' "$log"
            grep -q '^bios-ramfs shell smoke FAILED' "$log"
        else
            grep -q '^bios-disk shell smoke FAILED' "$log"
        fi
        ! grep -q '^uefi-.* shell smoke passed' "$log"
    elif [[ "$scenario" == success || "$scenario" == trace-success || "$scenario" == probe-success || "$scenario" == cloexec-success ]]; then
        [[ $status == 0 ]]
        grep -q '^bios userspace smoke passed$' "$log"
        grep -q '^uefi userspace smoke passed$' "$log"
        count=18
        [[ "$scenario" != probe-success ]] || count=20
        [[ "$scenario" != cloexec-success ]] || count=20
        [[ $(wc -l < "$state/commands") == "$count" ]]
    elif [[ "$scenario" == mutate-success ]]; then
        [[ $status == 1 ]]
        grep -q '^uefi userspace smoke passed$' "$log"
    else
        [[ $status == 1 ]]
        grep -q '^bios userspace smoke FAILED' "$log"
        ! grep -q 'userspace smoke passed' "$log"
    fi
    case "$scenario" in
        missing-shell|timeout|closed-output) grep -q 'timeout after 3s' "$log" ;;
        command-failure) grep -q 'command failed: /bin/busybox true' "$log" ;;
        echo-only) grep -q 'command failed: /bin/busybox echo KRUST_ECHO_OK' "$log" ;;
        bad-pwd) grep -q 'command failed: /bin/busybox pwd' "$log" ;;
        bad-cat) grep -q 'command failed: /bin/busybox cat /proc/meminfo' "$log" ;;
        probe-missing-address) grep -q 'cannot derive kernel mapping address' "$log" ;;
        probe-missing-marker) grep -q 'command failed: /bin/krust-mmap-probe' "$log" ;;
        cloexec-missing-*|cloexec-echo-only) grep -q 'command failed: cloexec-test' "$log" ;;
        shell-timeout) grep -q 'timeout after 3s' "$log" ;;
        shell-shutdown-error) grep -q 'unexpected shutdown output' "$log" ;;
    esac
    if [[ "$scenario" == mutate-* ]]; then
        grep -q '^Source VHD changed!$' "$log"
        [[ "$(sha256sum "$state/disk.vhd")" != "$before" ]]
    else
        grep -q '^Source VHD unchanged$' "$log"
        [[ "$(sha256sum "$state/disk.vhd")" == "$before" ]]
    fi
    while IFS= read -r dir; do
        [[ "$dir" == "$PWD"/target/userspace-smoke-* && ! -e "$dir" ]]
    done < "$state/workdirs"
    while IFS= read -r pid; do
        if kill -0 "$pid" 2>/dev/null; then
            echo "$scenario left owned PID $pid running" >&2
            exit 1
        fi
    done < "$state/pids"
    kill -0 "$sentinel"
    echo "$scenario passed (status $status; processes reaped, scratch removed)"
done

for invalid in '' 0 121 -1 1.5 03 '1s' '1;exit'; do
    status=0
    KRUST_SMOKE_TIMEOUT_SECONDS="$invalid" scripts/smoke-userspace.sh \
        "$work/success/disk.vhd" "$runner" > "$work/invalid.log" 2>&1 || status=$?
    [[ $status == 2 ]]
    grep -q 'must be an integer from 1 to 120' "$work/invalid.log"
done
echo "invalid deadlines rejected"

for invalid in missing-disk directory-disk missing-runner nonexecutable-runner extra-argument; do
    args=("$work/success/disk.vhd" "$runner")
    case "$invalid" in
        missing-disk) args[0]="$work/absent.vhd" ;;
        directory-disk) args[0]="$work" ;;
        missing-runner) args[1]="$work/absent-runner" ;;
        nonexecutable-runner) args[1]="$work/success/disk.vhd" ;;
        extra-argument) args+=(unexpected) ;;
    esac
    status=0
    KRUST_SMOKE_TIMEOUT_SECONDS=3 scripts/smoke-userspace.sh "${args[@]}" \
        > "$work/invalid.log" 2>&1 || status=$?
    [[ $status != 0 ]]
done
echo "invalid paths and arguments rejected"

for invalid in '' 2 true; do
    status=0
    KRUST_SMOKE_SHELL_PROBE="$invalid" scripts/smoke-userspace.sh \
        "$work/success/disk.vhd" "$runner" > "$work/invalid.log" 2>&1 || status=$?
    [[ $status == 2 ]]
    grep -q 'KRUST_SMOKE_SHELL_PROBE must be 0 or 1' "$work/invalid.log"
done
status=0
KRUST_SMOKE_SHELL_PROBE=1 KRUST_SMOKE_MMAP_PROBE=1 scripts/smoke-userspace.sh \
    "$work/success/disk.vhd" "$runner" > "$work/invalid.log" 2>&1 || status=$?
[[ $status == 2 ]]
grep -q 'Run the shell suite separately' "$work/invalid.log"
echo "invalid shell suite settings rejected"
