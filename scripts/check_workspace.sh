#!/usr/bin/env bash
# Run `cargo metadata --locked` and `cargo check` across the full workspace.
#
# This is a thin wrapper kept so contributors and CI can call the same
# command. Before compiling it asserts the release overflow policy
# (`overflow-checks = true` in every shipping release profile, see
# scripts/check-overflow-checks.sh) so the documented overflow behaviour
# cannot be silently disabled. The metadata check validates that the root
# Cargo.lock resolves cleanly without package collisions (e.g. duplicate
# workspace paths). Builds are `--locked` so dependency resolution stays
# pinned by the committed Cargo.lock. Extra arguments are forwarded to
# cargo, e.g.:
#
#   scripts/check_workspace.sh --all-targets
#   scripts/check_workspace.sh --release
set -euo pipefail

cd "$(dirname "$0")/.."

# Fail before compiling when a release profile dropped overflow checks.
scripts/check-overflow-checks.sh

# Fail before compiling when the root lockfile has package collisions.
cargo metadata --locked --format-version 1 >/dev/null

exec cargo check --workspace --locked "$@"
