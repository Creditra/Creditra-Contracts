#!/usr/bin/env bash
# Build both Soroban contracts to wasm32-unknown-unknown release artifacts.
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

case "$SELECTOR" in
    all)
        cargo build --target "$TARGET" --profile "$PROFILE" --workspace
        ;;
    credit)
        cargo build --target "$TARGET" --profile "$PROFILE" \
            -p creditra-credit
        ;;
    auction)
        cargo build --target "$TARGET" --profile "$PROFILE" \
            -p gateway-auction
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
