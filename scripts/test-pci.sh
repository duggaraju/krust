#!/usr/bin/env bash
# Test the kernel's actual hardware-independent scanner without privileged I/O.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p target
rustc --edition=2024 --test kernel/src/drivers/bus/pci/enumeration.rs \
    -o target/pci-enumeration-tests
target/pci-enumeration-tests
