#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../contracts/creditra-credit"

echo "Generating schema..."
cargo run --bin schema --locked

if [[ -n "$(git status --porcelain schema/)" ]]; then
    echo "::error::Schema is out of date. Run 'cargo run --bin schema' locally and commit the changes."
    git diff schema/
    exit 1
fi

echo "Schema is up to date."
