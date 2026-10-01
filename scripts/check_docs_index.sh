#!/usr/bin/env bash
#
# Fail when a docs/*.md document is not referenced from docs/INDEX.md.
#
# Every document must be discoverable from the index, otherwise it silently
# rots. This guard is also wired into CI.
#
# Usage:
#   scripts/check_docs_index.sh [--docs-dir DIR] [--index FILE] [--quiet]
#
# Exit codes:
#   0  every document is referenced
#   1  at least one document is missing from the index
#   2  usage / missing file error
set -euo pipefail

DOCS_DIR="docs"
INDEX_FILE=""
QUIET=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --docs-dir) DOCS_DIR="${2:-}"; shift 2 ;;
    --index) INDEX_FILE="${2:-}"; shift 2 ;;
    --quiet) QUIET=1; shift ;;
    -h|--help) sed -n '2,14p' "$0" | sed 's/^# *//'; exit 0 ;;
    *) echo "check_docs_index: unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [[ -z "$INDEX_FILE" ]]; then
  INDEX_FILE="$DOCS_DIR/INDEX.md"
fi

if [[ ! -d "$DOCS_DIR" ]]; then
  echo "check_docs_index: docs directory '$DOCS_DIR' not found" >&2
  exit 2
fi

if [[ ! -f "$INDEX_FILE" ]]; then
  echo "check_docs_index: index file '$INDEX_FILE' not found" >&2
  exit 2
fi

index_name="$(basename "$INDEX_FILE")"
missing=()

while IFS= read -r path; do
  name="$(basename "$path")"
  [[ "$name" == "$index_name" ]] && continue
  if ! grep -Fq "$name" "$INDEX_FILE"; then
    missing+=("$name")
  fi
done < <(find "$DOCS_DIR" -maxdepth 1 -type f -name '*.md' | sort)

if (( ${#missing[@]} > 0 )); then
  echo "check_docs_index: ${#missing[@]} document(s) not referenced from $INDEX_FILE:" >&2
  printf '  - %s\n' "${missing[@]}" >&2
  echo "Add each file to the \"Complete document inventory\" table." >&2
  exit 1
fi

if (( QUIET == 0 )); then
  echo "check_docs_index: all $DOCS_DIR/*.md documents are referenced from $INDEX_FILE"
fi
