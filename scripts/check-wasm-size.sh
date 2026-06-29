#!/usr/bin/env bash
# Fail when any workspace contract WASM exceeds the size budget (default 100 KB).
#
# Usage:
#   scripts/check-wasm-size.sh              # build all workspace WASM, then check
#   scripts/check-wasm-size.sh --check-only # check existing artifacts only
#
# Environment:
#   THRESHOLD_BYTES  Override limit in bytes (default: 102400 = 100 KiB)
#   WASM_DIR         Directory to scan (default: target/wasm32-unknown-unknown/release)
set -euo pipefail

cd "$(dirname "$0")/.."

CHECK_ONLY=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --check-only)
            CHECK_ONLY=1
            shift
            ;;
        -h | --help)
            sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "unknown argument: $1" >&2
            echo "usage: scripts/check-wasm-size.sh [--check-only]" >&2
            exit 64
            ;;
    esac
done

THRESHOLD_BYTES="${THRESHOLD_BYTES:-102400}"
WASM_DIR="${WASM_DIR:-target/wasm32-unknown-unknown/release}"

file_size_bytes() {
    local path="$1"
    if stat --format="%s" "$path" >/dev/null 2>&1; then
        stat --format="%s" "$path"
    elif stat -f "%z" "$path" >/dev/null 2>&1; then
        stat -f "%z" "$path"
    else
        wc -c <"$path" | tr -d '[:space:]'
    fi
}

if [[ "$CHECK_ONLY" -eq 0 ]]; then
    scripts/build_wasm.sh all
fi

if [[ ! -d "$WASM_DIR" ]]; then
    echo "::error::WASM directory not found: $WASM_DIR" >&2
    exit 1
fi

mapfile -t WASM_FILES < <(
    find "$WASM_DIR" -maxdepth 1 -name '*.wasm' -type f | sort
)

if [[ ${#WASM_FILES[@]} -eq 0 ]]; then
    echo "::error::No WASM artifacts found in $WASM_DIR" >&2
    exit 1
fi

echo "Threshold: ${THRESHOLD_BYTES} bytes ($((THRESHOLD_BYTES / 1024)) KB)"
echo "Scanning ${#WASM_FILES[@]} artifact(s) in ${WASM_DIR}"

fail=0
for wasm_path in "${WASM_FILES[@]}"; do
    size_bytes="$(file_size_bytes "$wasm_path")"
    wasm_name="$(basename "$wasm_path")"
    echo "${wasm_name}: ${size_bytes} bytes ($((size_bytes / 1024)) KB)"

    if [[ "$size_bytes" -gt "$THRESHOLD_BYTES" ]]; then
        echo "::error::${wasm_name} size ${size_bytes} exceeds threshold ${THRESHOLD_BYTES}." >&2
        fail=1
    else
        echo "::notice::${wasm_name} is ${size_bytes} bytes (within ${THRESHOLD_BYTES} byte budget)."
    fi
done

if [[ "$fail" -ne 0 ]]; then
    exit 1
fi

echo "All ${#WASM_FILES[@]} WASM artifact(s) within ${THRESHOLD_BYTES} byte budget."
