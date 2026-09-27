#!/usr/bin/env bash
# Focused tests for scripts/check-toolchain.sh (no toolchain install required).
set -euo pipefail

cd "$(dirname "$0")/.."

ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT

CHECK="$PWD/scripts/check-toolchain.sh"

PIN="9.9.9" # fixture pin, independent of the repo's real channel

# Valid rust-toolchain.toml fixture.
write_toml() {
    local channel="$1"
    cat > "$ROOT/rust-toolchain.toml" <<EOF
[toolchain]
channel = "$channel"
targets = ["wasm32-unknown-unknown"]
components = ["rustfmt", "clippy"]
EOF
}

# Valid CI workflow fixture: derives its toolchain from rust-toolchain.toml.
write_workflow() {
    local body="$1"
    printf '%s\n' "$body" > "$ROOT/ci.yml"
}

VALID_WORKFLOW='
- uses: dtolnay/rust-toolchain@master
  with:
    toolchain: ${{ steps.toolchain.outputs.channel }} # from rust-toolchain.toml
'

run_check() {
    bash "$CHECK" \
        --file "$ROOT/rust-toolchain.toml" \
        --workflow "$ROOT/ci.yml" \
        --lock "$ROOT/Cargo.lock" \
        > /dev/null 2>&1
}

assert_fails() {
    if run_check; then
        echo "expected check-toolchain to fail: $1" >&2
        exit 1
    fi
}

assert_passes() {
    if ! run_check; then
        echo "expected check-toolchain to pass: $1" >&2
        exit 1
    fi
}

# --- happy path ---------------------------------------------------------------
write_toml "$PIN"
write_workflow "$VALID_WORKFLOW"
truncate -s 10 "$ROOT/Cargo.lock" # outside git: existence is enough
assert_passes "valid pin + workflow + lock"

# Channel with inline comment and padded whitespace (boundary parse case)
cat > "$ROOT/rust-toolchain.toml" <<EOF
[toolchain]
channel = "$PIN"  # pinned by scripts/check-toolchain.sh policy
targets = ["wasm32-unknown-unknown"]
components = ["rustfmt", "clippy"]
EOF
assert_passes "channel line with trailing comment"

# --- floating / malformed channels --------------------------------------------
for floating in stable beta nightly nightly-2025-01-01; do
    write_toml "$floating"
    assert_fails "floating channel '$floating'"
done

write_toml "1.98" # two-component version is not an exact pin
assert_fails "non-semver channel"

write_toml "1.98.0.1" # four-component version is not an exact pin
assert_fails "over-specified channel"

# Missing channel entry
cat > "$ROOT/rust-toolchain.toml" <<EOF
[toolchain]
targets = ["wasm32-unknown-unknown"]
components = ["rustfmt", "clippy"]
EOF
assert_fails "missing channel entry"

# Duplicate channel entries (ambiguous pin)
cat > "$ROOT/rust-toolchain.toml" <<EOF
[toolchain]
channel = "1.98.0"
channel = "1.97.0"
targets = ["wasm32-unknown-unknown"]
components = ["rustfmt", "clippy"]
EOF
assert_fails "duplicate channel entries"

# --- required targets / components ---------------------------------------------
write_toml "$PIN"
cat > "$ROOT/rust-toolchain.toml" <<EOF
[toolchain]
channel = "$PIN"
components = ["rustfmt", "clippy"]
EOF
assert_fails "missing wasm32 target"

cat > "$ROOT/rust-toolchain.toml" <<EOF
[toolchain]
channel = "$PIN"
targets = ["wasm32-unknown-unknown"]
components = ["rustfmt"]
EOF
assert_fails "missing clippy component"

# --- toolchain file presence ----------------------------------------------------
rm -f "$ROOT/rust-toolchain.toml"
assert_fails "missing rust-toolchain.toml"
write_toml "$PIN"

# --- CI workflow policy ----------------------------------------------------------
write_workflow '
- uses: dtolnay/rust-toolchain@stable
'
assert_fails "workflow with @stable ref"

write_workflow '
- uses: dtolnay/rust-toolchain@master
  with:
    toolchain: stable
'
assert_fails "workflow with floating toolchain input"

write_workflow '
- uses: dtolnay/rust-toolchain@master
  with:
    toolchain: stable minus 8 releases
'
assert_fails "workflow with sliding-window toolchain expression"

write_workflow '
- uses: dtolnay/rust-toolchain@master
'
assert_fails "workflow that bypasses rust-toolchain.toml"

# A literal pinned version in the workflow is allowed (no floating channel).
write_workflow '
- uses: dtolnay/rust-toolchain@master
  with:
    toolchain: '"$PIN"' # from rust-toolchain.toml
'
assert_passes "workflow with literal pinned toolchain input"

# Workflow checks can be skipped entirely.
if ! bash "$CHECK" \
    --file "$ROOT/rust-toolchain.toml" \
    --workflow "$ROOT/does-not-exist.yml" \
    --skip-workflow \
    --lock "$ROOT/Cargo.lock" > /dev/null 2>&1; then
    echo "expected check-toolchain to pass with --skip-workflow" >&2
    exit 1
fi

# --- lock file policy -------------------------------------------------------------
rm -f "$ROOT/Cargo.lock"
assert_fails "missing lock file"

# Uncommitted lock inside a git repo must fail; committed lock must pass.
git init -q "$ROOT/gitrepo"
touch "$ROOT/gitrepo/Cargo.lock"
if bash "$CHECK" \
    --file "$ROOT/rust-toolchain.toml" \
    --skip-workflow \
    --lock "$ROOT/gitrepo/Cargo.lock" > /dev/null 2>&1; then
    echo "expected check-toolchain to fail: uncommitted lock file" >&2
    exit 1
fi
git -C "$ROOT/gitrepo" add Cargo.lock
if ! bash "$CHECK" \
    --file "$ROOT/rust-toolchain.toml" \
    --skip-workflow \
    --lock "$ROOT/gitrepo/Cargo.lock" > /dev/null 2>&1; then
    echo "expected check-toolchain to pass: committed lock file" >&2
    exit 1
fi
truncate -s 10 "$ROOT/Cargo.lock"

# --- --verify-active ----------------------------------------------------------------
STUB="$ROOT/bin"
mkdir -p "$STUB"

verify_with_rustc() {
    local version_output="$1"
    printf '#!/usr/bin/env bash\necho "%s"\n' "$version_output" > "$STUB/rustc"
    chmod +x "$STUB/rustc"
    PATH="$STUB:$PATH" bash "$CHECK" \
        --file "$ROOT/rust-toolchain.toml" \
        --skip-workflow \
        --lock "$ROOT/Cargo.lock" \
        --verify-active > /dev/null 2>&1
}

if ! verify_with_rustc "rustc $PIN (abc123 2025-01-01)"; then
    echo "expected --verify-active to pass when rustc matches the pin" >&2
    exit 1
fi

if verify_with_rustc "rustc 9.9.8 (def456 2025-01-01)"; then
    echo "expected --verify-active to fail when rustc does not match the pin" >&2
    exit 1
fi

if verify_with_rustc "rustc 9.9.9-dev (def456 2025-01-01)"; then
    echo "expected --verify-active to fail on a near-miss version" >&2
    exit 1
fi

# No rustc on PATH at all must fail with a clear message.
if PATH="/nonexistent" bash "$CHECK" \
    --file "$ROOT/rust-toolchain.toml" \
    --skip-workflow \
    --lock "$ROOT/Cargo.lock" \
    --verify-active > /dev/null 2>&1; then
    echo "expected --verify-active to fail when rustc is absent" >&2
    exit 1
fi

# --- usage errors ---------------------------------------------------------------------
if bash "$CHECK" --nonsense > /dev/null 2>&1; then
    echo "expected usage error exit for unknown argument" >&2
    exit 1
fi
if bash "$CHECK" --file > /dev/null 2>&1; then
    echo "expected usage error exit for --file without a path" >&2
    exit 1
fi

echo "check-toolchain guard tests passed"
