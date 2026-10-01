#!/usr/bin/env bash
# Fail when a release profile drops `overflow-checks = true`.
#
# The deployed release WASM must revert on arithmetic overflow instead of
# wrapping, because not every `i128` accounting path routes through `checked_*`
# (e.g. `already_reversed + amount` in `reverse_draw`). README.md and the
# credit contract crate docs state that the release profile enables overflow
# checks, so this guard turns that statement into an enforced invariant: it
# fails fast when the key is missing or flipped back to `false`.
#
# Profiles checked (both ship contract WASM):
#   Cargo.toml                            [profile.release]  (workspace)
#   contracts/creditra-credit/Cargo.toml  [profile.release]  (standalone crate)
#
# Usage:
#   scripts/check-overflow-checks.sh                  # check both manifests
#   scripts/check-overflow-checks.sh --manifest PATH  # check a specific file
#   scripts/check-overflow-checks.sh -h | --help
#
# Exit codes:
#   0   every release profile sets overflow-checks = true
#   1   a release profile omits or disables overflow checks
#   64  usage error
set -euo pipefail

# Resolve our own path before changing directory so `--help` works when the
# script is invoked via a relative path from any working directory.
SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SELF_PATH="$SELF_DIR/$(basename "${BASH_SOURCE[0]}")"
cd "$SELF_DIR/.."

MANIFESTS=(
    "Cargo.toml"
    "contracts/creditra-credit/Cargo.toml"
)

usage() {
    sed -n '2,23p' "$SELF_PATH" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --manifest)
            [[ $# -ge 2 ]] || { echo "--manifest requires a path" >&2; exit 64; }
            MANIFESTS=("$2")
            shift 2
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            echo "unknown argument: $1" >&2
            echo "usage: scripts/check-overflow-checks.sh [--manifest PATH]" >&2
            exit 64
            ;;
    esac
done

# Echo the value assigned to `overflow-checks` inside the [profile.release]
# table of a manifest, or nothing when the table or key is absent. Comments
# and surrounding whitespace are stripped so `overflow-checks = true # note`
# and `overflow-checks=true` both normalise to `true`.
release_overflow_checks() {
    local manifest="$1"
    awk '
        /^[[:space:]]*\[[^]]*\][[:space:]]*$/ {
            in_release = ($0 ~ /^[[:space:]]*\[profile\.release\][[:space:]]*$/)
            next
        }
        in_release && /^[[:space:]]*overflow-checks[[:space:]]*=/ {
            line = $0
            sub(/^[^=]*=[[:space:]]*/, "", line)
            sub(/[[:space:]]*(#.*)?$/, "", line)
            print line
            exit
        }
    ' "$manifest"
}

fail=0

for manifest in "${MANIFESTS[@]}"; do
    if [[ ! -f "$manifest" ]]; then
        echo "::error::Manifest not found: $manifest" >&2
        fail=1
        continue
    fi

    value="$(release_overflow_checks "$manifest")"
    if [[ -z "$value" ]]; then
        echo "::error::$manifest has no 'overflow-checks' key in [profile.release]; release WASM would wrap on overflow." >&2
        echo "Add 'overflow-checks = true' to [profile.release] so i128 accounting reverts instead of wrapping." >&2
        fail=1
    elif [[ "$value" != "true" ]]; then
        echo "::error::$manifest sets overflow-checks = $value in [profile.release], expected true." >&2
        echo "Release WASM must revert on arithmetic overflow; see the README release-profile notes." >&2
        fail=1
    else
        echo "$manifest: [profile.release] overflow-checks = true"
    fi
done

if [[ "$fail" -ne 0 ]]; then
    echo "::error::Release overflow-check policy violated. CI FAILED." >&2
    exit 1
fi

echo "Release overflow-check policy OK: overflow-checks = true in every checked release profile."
exit 0
