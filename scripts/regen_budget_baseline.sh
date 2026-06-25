#!/bin/bash
set -e

echo "Regenerating Soroban budget baselines..."
cargo run --example budget_baseline
echo "Baselines updated in contracts/credit/test_snapshots/budget.json"
