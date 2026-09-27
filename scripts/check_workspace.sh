#!/usr/bin/env bash
# Run `cargo metadata --locked` and `cargo check` across the full workspace.
#
# The metadata check validates that the root Cargo.lock resolves cleanly
# without package collisions (e.g. duplicate workspace paths). Builds are
# `--locked` so dependency resolution stays pinned by the committed
# Cargo.lock. Extra arguments are forwarded to cargo, e.g.:
#
#   scripts/check_workspace.sh --all-targets
#   scripts/check_workspace.sh --release
set -euo pipefail

cd "$(dirname "$0")/.."

cargo metadata --locked --format-version 1

exec cargo check --workspace --locked "$@"
