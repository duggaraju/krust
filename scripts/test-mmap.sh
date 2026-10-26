#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p target
work="$PWD/target/mmap-range-tests-$$"
mkdir "$work"
trap 'rm -rf -- "$work"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
export TMPDIR="$work"
rustc --edition=2024 --test scripts/tests/mmap-range.rs -o "$work/tests"
"$work/tests"
rustc --edition=2024 --crate-type=lib --emit=metadata --target aarch64-unknown-none \
    scripts/tests/mmap-aarch64-surface.rs -o "$work/aarch64.rmeta"
