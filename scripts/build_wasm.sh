#!/usr/bin/env bash
# Build both Soroban contracts to wasm32-unknown-unknown release artifacts.
#
# Reproducibility: the script first enforces the toolchain pin policy
# (scripts/check-toolchain.sh --verify-active) so a drifting rustc cannot
# silently produce different artifacts, and compiles `--locked` so dependency
# resolution is pinned by the committed Cargo.lock.
#
# Usage:
#   scripts/build_wasm.sh            # builds all workspace contracts
#   scripts/build_wasm.sh credit     # builds only creditra-credit
#   scripts/build_wasm.sh auction    # builds only gateway-auction
#
# Output: target/wasm32-unknown-unknown/release/*.wasm
set -euo pipefail

cd "$(dirname "$0")/.."

TARGET="wasm32-unknown-unknown"
PROFILE="release"
SELECTOR="${1:-all}"

scripts/check-toolchain.sh --verify-active

case "$SELECTOR" in
    all)
        cargo build --target "$TARGET" --profile "$PROFILE" --workspace --locked
        ;;
    credit)
        cargo build --target "$TARGET" --profile "$PROFILE" \
            --locked -p creditra-credit
        ;;
    auction)
        cargo build --target "$TARGET" --profile "$PROFILE" \
            --locked -p gateway-auction
        ;;
    *)
        echo "unknown selector: $SELECTOR" >&2
        echo "expected one of: all, credit, auction" >&2
        exit 64
        ;;
esac

echo
echo "WASM artifacts:"
find target/"$TARGET"/"$PROFILE" -maxdepth 1 -name '*.wasm' -print 2>/dev/null || true
