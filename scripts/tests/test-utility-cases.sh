#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
source scripts/tests/utility-cases.sh
binary=$(realpath -e "${1:-target/rootfs/bin/busybox}")
work=$(mktemp -d "$PWD/target/utility-cases-XXXXXX")
trap 'rm -rf -- "$work"' EXIT
bash scripts/install-busybox-applets.sh "$binary" "$work" ext4
count=0
for path in "$work/bin/"*; do
    name=${path##*/}
    [[ "$name" != busybox ]] || continue
    utility_case "$name"
    [[ -n "$utility_command" ]]
    case "$utility_check" in
        exact) sample=$utility_expected ;;
        listing) sample='input repeat one bytes strings data.gz data.tar empty' ;;
        wc) sample='  3 /fixture/input' ;;
        ps) sample=$'PID USER COMMAND\n1 root /bin/busybox' ;;
        number) sample=123456 ;;
        od) sample=' 41 42' ;;
        network)
            echo "$name needs a guest HTTP fixture before it has a passing validator" >&2
            exit 1 ;;
        *) echo "Missing validator sample for $name" >&2; exit 1 ;;
    esac
    original_name=$name
    utility_validate "$sample" "$utility_status"
    [[ "$name" == "$original_name" ]]
    if utility_validate "$sample" 99; then
        echo "$name accepted an incorrect exit status" >&2
        exit 1
    fi
    if utility_validate INVALID "$utility_status"; then
        echo "$name accepted incorrect output" >&2
        exit 1
    fi
    count=$((count + 1))
done
if utility_case unknown 2>/dev/null; then
    echo "An unknown utility was accepted without a functional case" >&2
    exit 1
fi
echo "$count installed utility validators passed positive and negative checks"
