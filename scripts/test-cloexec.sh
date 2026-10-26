#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p target
work="$PWD/target/cloexec-tests-$$"
mkdir "$work"
trap 'rm -rf -- "$work"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
export TMPDIR="$work"
rustc --edition=2024 --test scripts/tests/cloexec.rs -o "$work/tests"
"$work/tests"
