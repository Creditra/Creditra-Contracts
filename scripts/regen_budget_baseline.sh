#!/usr/bin/env bash
# scripts/regen_budget_baseline.sh
#
# Regenerate contracts/credit/test_snapshots/budget.json from live measurements.
#
# Usage:
#   ./scripts/regen_budget_baseline.sh          # regenerate and show diff
#   ./scripts/regen_budget_baseline.sh --no-diff # skip diff (CI bootstrap)
#
# The script runs the `budget_baseline` example inside the `credit` crate,
# which calls every instrumented entrypoint with the same setup used by
# tests/budget_regression.rs and overwrites the snapshot file.
#
# Review the diff — committing inflated numbers defeats the purpose.

set -euo pipefail

CRATE="contracts/credit"
SNAPSHOT="${CRATE}/test_snapshots/budget.json"
SHOW_DIFF=true

for arg in "$@"; do
  case "$arg" in
    --no-diff) SHOW_DIFF=false ;;
    *) echo "Unknown argument: $arg"; exit 1 ;;
  esac
done

echo "==> Building and running budget_baseline example …"
cargo run \
  --manifest-path "${CRATE}/Cargo.toml" \
  --features instrument \
  --example budget_baseline \
  2>&1

echo ""
echo "==> Snapshot written to: ${SNAPSHOT}"

if $SHOW_DIFF; then
  if git diff --quiet -- "${SNAPSHOT}" 2>/dev/null; then
    echo "    No changes detected — baselines are up to date."
  else
    echo ""
    echo "==> Diff (review before committing):"
    git diff -- "${SNAPSHOT}" || true
  fi
fi

echo ""
echo "Done.  If the numbers look correct, commit with:"
echo "  git add ${SNAPSHOT}"
echo "  git commit -m 'test: regen budget baselines'"