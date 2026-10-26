#!/usr/bin/env bash
# Share boot, prompt, deadline, snapshot, and cleanup handling with userspace smoke.
set -euo pipefail
cd "$(dirname "$0")/.."
export KRUST_SMOKE_SHELL_PROBE=1
exec bash scripts/smoke-userspace.sh "$@"
