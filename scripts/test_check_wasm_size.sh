#!/usr/bin/env bash
# Focused tests for scripts/check-wasm-size.sh (no contract build required).
set -euo pipefail

cd "$(dirname "$0")/.."

ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT

CHECK="$PWD/scripts/check-wasm-size.sh"
THRESHOLD=102400

run_check() {
    THRESHOLD_BYTES="$THRESHOLD" WASM_DIR="$ROOT" bash "$CHECK" --check-only
}

assert_fails() {
    if THRESHOLD_BYTES="$THRESHOLD" WASM_DIR="$ROOT" bash "$CHECK" --check-only >/dev/null 2>&1; then
        echo "expected check-wasm-size to fail: $1" >&2
        exit 1
    fi
}

# Under budget
truncate -s 100 "$ROOT/small.wasm"
run_check

# Exactly at budget (inclusive limit)
rm -f "$ROOT"/*.wasm
truncate -s "$THRESHOLD" "$ROOT/exact.wasm"
run_check

# One byte over budget
rm -f "$ROOT"/*.wasm
truncate -s $((THRESHOLD + 1)) "$ROOT/too_large.wasm"
assert_fails "single oversized artifact"

# Multiple artifacts: fail if any one exceeds budget
rm -f "$ROOT"/*.wasm
truncate -s 100 "$ROOT/ok.wasm"
truncate -s $((THRESHOLD + 1)) "$ROOT/bad.wasm"
assert_fails "mixed pass/fail artifacts"

# Empty directory
rm -f "$ROOT"/*.wasm
assert_fails "empty wasm directory"

echo "check-wasm-size guard tests passed"
