#!/usr/bin/env bash
# Enforce the reproducible-build policy for the Creditra contracts workspace.
#
# Builds are only reproducible across machines and over time when every
# environment compiles with the *same* pinned toolchain and resolves
# dependencies from the same committed lock files. This guard fails fast on
# the drift that silently breaks that property:
#
#   1. `rust-toolchain.toml` must pin `channel` to an exact `X.Y.Z` version.
#      Floating channels (`stable`, `beta`, `nightly`, date-suffixed
#      variants) resolve to different compilers on different days and are
#      rejected.
#   2. `wasm32-unknown-unknown` and the `rustfmt` / `clippy` components must
#      stay declared in the toolchain file so local and CI environments stay
#      identical.
#   3. The CI workflow must consume the pin (reference `rust-toolchain.toml`)
#      and must not select a floating toolchain (`@stable` refs or floating
#      `toolchain:` inputs).
#   4. Every required `Cargo.lock` must exist and be committed to git.
#   5. With `--verify-active`, the currently active `rustc` must match the
#      pin — catching stray `rustup override`s, `RUSTUP_TOOLCHAIN` values, or
#      rustup-less environments before they produce diverging artifacts.
#
# Usage:
#   scripts/check-toolchain.sh                 # policy check (default paths)
#   scripts/check-toolchain.sh --verify-active # policy + active-rustc check
#   scripts/check-toolchain.sh --file <rust-toolchain.toml> \
#       --workflow <ci.yml> --lock <Cargo.lock> [--lock <more.lock> ...]
#   scripts/check-toolchain.sh -h | --help
#
# Exit codes:
#   0   all checks passed
#   1   reproducibility policy violation
#   64  usage error
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/.." && pwd)"

TOML_FILE="$REPO_ROOT/rust-toolchain.toml"
WORKFLOW_FILE="$REPO_ROOT/.github/workflows/ci.yml"
VERIFY_ACTIVE=0
LOCK_FILES=("Cargo.lock") # relative paths resolved against the caller's cwd

usage() {
    sed -n '2,40p' "$0" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --verify-active)
            VERIFY_ACTIVE=1
            shift
            ;;
        --file)
            [[ $# -ge 2 ]] || { echo "--file requires a path" >&2; exit 64; }
            TOML_FILE="$2"
            shift 2
            ;;
        --workflow)
            [[ $# -ge 2 ]] || { echo "--workflow requires a path" >&2; exit 64; }
            WORKFLOW_FILE="$2"
            shift 2
            ;;
        --lock)
            [[ $# -ge 2 ]] || { echo "--lock requires a path" >&2; exit 64; }
            LOCK_FILES+=("$2")
            shift 2
            ;;
        --skip-workflow)
            WORKFLOW_FILE=""
            shift
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            echo "unknown argument: $1" >&2
            echo "usage: scripts/check-toolchain.sh [--verify-active] [--file PATH] [--workflow PATH] [--skip-workflow] [--lock PATH]..." >&2
            exit 64
            ;;
    esac
done

fail=0

# --- 1. Toolchain file must pin an exact version -----------------------------
if [[ ! -f "$TOML_FILE" ]]; then
    echo "::error::Toolchain file not found: $TOML_FILE" >&2
    echo "Reproducible builds require a pinned rust-toolchain.toml at the repo root." >&2
    exit 1
fi

channel_lines="$(grep -cE '^[[:space:]]*channel[[:space:]]*=' "$TOML_FILE" || true)"
if [[ "$channel_lines" -ne 1 ]]; then
    echo "::error::Expected exactly one 'channel =' entry in $TOML_FILE, found $channel_lines." >&2
    fail=1
    channel=""
else
    channel="$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$TOML_FILE" | head -n1)"
    if [[ -z "$channel" ]]; then
        echo "::error::Could not parse the 'channel' value in $TOML_FILE." >&2
        fail=1
    elif [[ "$channel" =~ ^(stable|beta|nightly)(-[0-9]{4}-[0-9]{2}-[0-9]{2})?$ ]]; then
        echo "::error::Toolchain channel '$channel' is floating: it resolves to a different compiler over time." >&2
        echo "Pin an exact version instead, e.g. channel = \"1.98.0\"." >&2
        fail=1
    elif ! [[ "$channel" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        echo "::error::Toolchain channel '$channel' is not an exact X.Y.Z version." >&2
        echo "Reproducible builds require an exact semver pin, e.g. channel = \"1.98.0\"." >&2
        fail=1
    fi
fi

# --- 2. Required targets and components --------------------------------------
if ! grep -q 'wasm32-unknown-unknown' "$TOML_FILE"; then
    echo "::error::$TOML_FILE must declare the 'wasm32-unknown-unknown' target so local and CI WASM builds match." >&2
    fail=1
fi
for component in rustfmt clippy; do
    if ! grep -q "$component" "$TOML_FILE"; then
        echo "::error::$TOML_FILE must declare the '$component' component so local and CI linting match." >&2
        fail=1
    fi
done

# --- 3. CI workflow must consume the pin, never a floating channel -----------
if [[ -n "$WORKFLOW_FILE" ]]; then
    if [[ ! -f "$WORKFLOW_FILE" ]]; then
        echo "::error::CI workflow not found: $WORKFLOW_FILE" >&2
        fail=1
    else
        if ! grep -q 'rust-toolchain.toml' "$WORKFLOW_FILE"; then
            echo "::error::$WORKFLOW_FILE does not reference rust-toolchain.toml; CI must derive its toolchain from the same pinned source as local builds." >&2
            fail=1
        fi
        if grep -nE 'rust-toolchain@(stable|beta|nightly)' "$WORKFLOW_FILE" >/dev/null; then
            echo "::error::$WORKFLOW_FILE installs a floating toolchain via rust-toolchain@stable|beta|nightly." >&2
            grep -nE 'rust-toolchain@(stable|beta|nightly)' "$WORKFLOW_FILE" >&2
            fail=1
        fi
        if grep -nE 'toolchain:[[:space:]]*(stable|beta|nightly)([^0-9.]|$)' "$WORKFLOW_FILE" >/dev/null; then
            echo "::error::$WORKFLOW_FILE passes a floating channel via the 'toolchain:' input." >&2
            grep -nE 'toolchain:[[:space:]]*(stable|beta|nightly)([^0-9.]|$)' "$WORKFLOW_FILE" >&2
            fail=1
        fi
    fi
fi

# --- 4. Lock files must exist and be committed -------------------------------
for lock in "${LOCK_FILES[@]}"; do
    if [[ ! -f "$lock" ]]; then
        echo "::error::Lock file missing: $lock. Deterministic dependency resolution requires a committed Cargo.lock." >&2
        fail=1
        continue
    fi
    lock_dir="$(dirname "$lock")"
    if git -C "$lock_dir" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        # git resolves pathspecs relative to the -C directory, so compare
        # against the lock file's own name inside that directory.
        if ! git -C "$lock_dir" ls-files --error-unmatch "$(basename "$lock")" >/dev/null 2>&1; then
            echo "::error::Lock file $lock exists but is not committed to git. Run: git add $lock" >&2
            fail=1
        fi
    fi
    # Lock files outside a git tree (test fixtures) only need to exist.
done

# --- 5. Active compiler must match the pin (optional, opt-in) ----------------
if [[ "$VERIFY_ACTIVE" -eq 1 && "$fail" -eq 0 ]]; then
    if ! command -v rustc >/dev/null 2>&1; then
        echo "::error::rustc not found on PATH; cannot verify the active toolchain against the pin." >&2
        fail=1
    else
        active_version="$(rustc --version | awk '{print $2}')"
        if [[ "$active_version" != "$channel" ]]; then
            echo "::error::Active rustc is $active_version but rust-toolchain.toml pins $channel." >&2
            echo "A stray rustup override, RUSTUP_TOOLCHAIN, or rustup-less install produces diverging builds." >&2
            echo "Fix with: rustup toolchain install $channel && rustup default $channel" >&2
            fail=1
        else
            echo "Active rustc $active_version matches the pinned channel."
        fi
    fi
fi

if [[ "$fail" -ne 0 ]]; then
    echo "::error::Reproducible-build policy violations found. CI FAILED." >&2
    exit 1
fi

echo "Reproducible-build policy OK: toolchain pinned to $channel, wasm target + components declared, lock files committed."
exit 0
