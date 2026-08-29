#!/usr/bin/env bash
# Run `cargo check` across the full workspace.
#
# This is a thin wrapper kept so contributors and CI can call the same
# command. Builds are `--locked` so dependency resolution stays pinned by the
# committed Cargo.lock. Extra arguments are forwarded to cargo, e.g.:
#
#   scripts/check_workspace.sh --all-targets
#   scripts/check_workspace.sh --release
set -euo pipefail

cd "$(dirname "$0")/.."

exec cargo check --workspace --locked "$@"
