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

for scenario in success missing-shell command-failure timeout closed-output echo-only bad-pwd bad-cat runner-failure mutate-failure mutate-success; do
    state="$work/$scenario"
    mkdir "$state"
    printf 'original VHD fixture\n' > "$state/disk.vhd"
    before=$(sha256sum "$state/disk.vhd")
    log="$PWD/target/userspace-smoke-test-$scenario.log"
    status=0
    started=$SECONDS
    SMOKE_TEST_STATE="$state" SMOKE_TEST_CASE="$scenario" KRUST_SMOKE_TIMEOUT_SECONDS=3 \
        scripts/smoke-userspace.sh "$state/disk.vhd" "$runner" > "$log" 2>&1 || status=$?
    (( SECONDS - started <= 8 ))
    if [[ "$scenario" == success ]]; then
        [[ $status == 0 ]]
        grep -q '^bios userspace smoke passed$' "$log"
        grep -q '^uefi userspace smoke passed$' "$log"
        [[ $(wc -l < "$state/commands") == 18 ]]
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
