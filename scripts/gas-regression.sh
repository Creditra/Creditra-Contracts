#!/usr/bin/env bash
# scripts/gas-regression.sh
#
# High-level orchestrator for the Creditra gas-regression workflow.
#
# Usage:
#   ./scripts/gas-regression.sh             # run the regression tests (CI)
#   ./scripts/gas-regression.sh --regen     # regenerate baselines first, then test
#   ./scripts/gas-regression.sh --regen-only  # regenerate baselines only
#
# By default the script runs `cargo test budget_regression` inside the `credit`
# crate to check observed resource usage against the pinned baselines in
# `contracts/credit/test_snapshots/budget.json`.
#
# When `--regen` (or `--regen-only`) is passed, it first re-runs the
# `budget_baseline` example to overwrite the snapshot with fresh numbers.
#
# Exit code
# ---------
# 0  – all checks passed (or regeneration completed)
# 1  – any sub-step failed

set -euo pipefail

CRATE="contracts/credit"
REBUILD=false
REBUILD_ONLY=false

for arg in "$@"; do
  case "$arg" in
    --regen)      REBUILD=true ;;
    --regen-only) REBUILD_ONLY=true ;;
    *) echo "Unknown argument: $arg"; exit 1 ;;
  esac
done

# ── (optional) regenerate baselines ─────────────────────────────────────────
if $REBUILD || $REBUILD_ONLY; then
  echo "==> Regenerating budget baselines …"
  cargo run \
    --manifest-path "${CRATE}/Cargo.toml" \
    --example budget_baseline \
    2>&1
  echo "    Done."
  echo ""
  if $REBUILD_ONLY; then
    echo "Baselines regenerated. Review the diff then commit."
    exit 0
  fi
fi

# ── run regression tests ──────────────────────────────────────────────────
echo "==> Running budget-regression tests …"
exec cargo test \
  --manifest-path "${CRATE}/Cargo.toml" \
  --test budget_regression \
  2>&1
