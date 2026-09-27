#!/usr/bin/env bash
# Build Soroban contracts to WASM and assert each is within ±5 KB of the
# checked-in baseline (scripts/wasm-size-baseline.txt).
#
# Usage:
#   scripts/check-wasm-baseline.sh              # build + check
#   scripts/check-wasm-baseline.sh --check-only # check existing artifacts only
#
# Exit codes:
#   0  All builds within tolerance
#   1  One or more builds exceed the baseline + tolerance
set -euo pipefail

cd "$(dirname "$0")/.."

BASELINE_FILE="scripts/wasm-size-baseline.txt"
TOLERANCE_BYTES="${TOLERANCE_BYTES:-5120}"  # ±5 KB
WASM_DIR="${WASM_DIR:-target/wasm32-unknown-unknown/release}"

CHECK_ONLY=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --check-only)
            CHECK_ONLY=1
            shift
            ;;
        -h | --help)
            sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "unknown argument: $1" >&2
            echo "usage: scripts/check-wasm-baseline.sh [--check-only]" >&2
            exit 64
            ;;
    esac
done

if [[ ! -f "$BASELINE_FILE" ]]; then
    echo "::error::Baseline file not found: $BASELINE_FILE" >&2
    exit 1
fi

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

# Build all workspace contracts unless --check-only
if [[ "$CHECK_ONLY" -eq 0 ]]; then
    scripts/build_wasm.sh all
fi

if [[ ! -d "$WASM_DIR" ]]; then
    echo "::error::WASM directory not found: $WASM_DIR" >&2
    exit 1
fi

echo "Tolerance: ±${TOLERANCE_BYTES} bytes ($((TOLERANCE_BYTES / 1024)) KB)"
echo

fail=0
warn=0
count=0

while IFS=' ' read -r crate_name size_bytes; do
    # Skip comments and blank lines
    [[ "$crate_name" =~ ^#.*$ || -z "$crate_name" ]] && continue

    count=$((count + 1))
    baseline="$size_bytes"
    wasm_path="${WASM_DIR}/${crate_name//-/_}.wasm"

    if [[ ! -f "$wasm_path" ]]; then
        echo "::error::WASM artifact not found for ${crate_name} at ${wasm_path}" >&2
        fail=1
        continue
    fi

    actual_bytes="$(file_size_bytes "$wasm_path")"
    upper=$((baseline + TOLERANCE_BYTES))
    lower=$((baseline - TOLERANCE_BYTES))
    # Prevent underflow for small baselines
    [[ "$lower" -lt 0 ]] && lower=0

    echo "${crate_name}:"
    echo "  baseline: ${baseline} bytes ($((baseline / 1024)) KB)"
    echo "  current:  ${actual_bytes} bytes ($((actual_bytes / 1024)) KB)"
    echo "  range:    [${lower}, ${upper}] bytes"

    if [[ "$actual_bytes" -gt "$upper" ]]; then
        echo "  status:   FAIL (over budget by $((actual_bytes - upper)) bytes)" >&2
        echo "::error::${crate_name} WASM size ${actual_bytes} exceeds baseline ${baseline} + tolerance ${TOLERANCE_BYTES}" >&2
        fail=1
    elif [[ "$actual_bytes" -lt "$lower" ]]; then
        echo "  status:   WARN (under budget by $((lower - actual_bytes)) bytes — update baseline?)"
        warn=1
    else
        echo "  status:   OK"
    fi
    echo
done < "$BASELINE_FILE"

if [[ "$count" -eq 0 ]]; then
    echo "::error::No baselines found in $BASELINE_FILE" >&2
    exit 1
fi

if [[ "$warn" -ne 0 ]]; then
    echo "::notice::Some contracts are under budget. Consider updating the baseline."
fi

if [[ "$fail" -ne 0 ]]; then
    echo "::error::One or more contracts exceed the size budget. CI FAILED." >&2
    exit 1
fi

echo "All ${count} contract(s) within ±${TOLERANCE_BYTES} byte tolerance of baseline."
