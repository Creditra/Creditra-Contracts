#!/usr/bin/env bash
# Remove stray *.profraw coverage profiles left behind by cargo llvm-cov
# or instrumented test runs.
#
# By default the workspace .gitignore excludes these, but they can pile up
# in nested workspace member directories and clutter `git status -uall`.
#
# Usage:
#   scripts/clean_profraw.sh           # delete everywhere under repo root
#   scripts/clean_profraw.sh --dry-run # show what would be deleted
set -euo pipefail

cd "$(dirname "$0")/.."

DRY_RUN=0
if [[ "${1:-}" == "--dry-run" ]]; then
    DRY_RUN=1
fi

count=0
while IFS= read -r -d '' file; do
    count=$((count + 1))
    if [[ $DRY_RUN -eq 1 ]]; then
        echo "would remove: $file"
    else
        rm -f -- "$file"
        echo "removed: $file"
    fi
done < <(find . -type f -name '*.profraw' -not -path './target/*' -print0)

if [[ $count -eq 0 ]]; then
    echo "no *.profraw files found"
fi
