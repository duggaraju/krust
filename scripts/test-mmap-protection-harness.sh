#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
work="$PWD/target/mmap-protection-tests-$$"
mkdir -p target
mkdir "$work"
trap 'rm -rf -- "$work"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
printf 'host fixture\n' > "$work/disk.vhd"
for scenario in success missing-fault missing-status wrong-fault unexpected-fault shutdown-fault; do
    status=0
    MMAP_TEST_CASE="$scenario" bash scripts/test-mmap-protections.sh \
        "$work/disk.vhd" "$PWD/scripts/tests/mmap-protection-runner.sh" \
        > "$work/result" 2>&1 || status=$?
    if [[ "$scenario" == success ]]; then
        [[ $status == 0 ]]
        grep -q '^bios mmap protections passed' "$work/result"
        grep -q '^uefi mmap protections passed' "$work/result"
    else
        [[ $status == 1 ]]
        grep -q '^bios mmap protections FAILED' "$work/result"
        ! grep -q 'mmap protections passed' "$work/result"
    fi
    echo "$scenario protection harness passed"
done
