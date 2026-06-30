#!/usr/bin/env bash
# scripts/wasm-size-baseline.sh
#
# Compare the workspace contract WASM sizes against the pinned baseline in
# `scripts/wasm-size-baseline.json`. Implements issue #492: per-crate WASM
# size budget guardrail that fails CI when a release build drifts more than
# the configured tolerance (5 KB by default) from the committed baseline.
#
# Usage
# -----
#   ./scripts/wasm-size-baseline.sh            # Default: --regen (re-measure + write)
#   ./scripts/wasm-size-baseline.sh --regen    # Same as above, explicit.
#   ./scripts/wasm-size-baseline.sh --no-diff  # Like --regen, suppress git diff output.
#   ./scripts/wasm-size-baseline.sh --check    # CI: enforce ±tolerance, do NOT write.
#
# Modes
# -----
# regen   Build the workspace in --release for `wasm32-unknown-unknown`,
#         measure each crate's `.wasm`, write the bytes back into
#         `scripts/wasm-size-baseline.json`, and (unless --no-diff) show
#         the resulting `git diff`. Use this locally to seed or update
#         the baseline after a legitimate size change.
#
# check   Build the workspace, then compare each crate's measured size
#         against `size_bytes` in the baseline.
#         - Delta > `tolerance_bytes` → fail (over-budget = regression).
#         - Delta < -`tolerance_bytes` → ::warning:: (under-budget; verify
#           a feature was not inadvertently dropped).
#         - 0 < |delta| <= `tolerance_bytes` → ::notice:: (within budget,
#           but with measurable drift; satisfies the issue-#492 "build
#           within the budget warns the user" acceptance criterion).
#         - `|delta| == 0` and `size_bytes == 0` (uninitialized baseline)
#           → hard error; CI cannot silently re-seed itself.
#
# Conventions
# -----------
# - Mirrors the style of `scripts/regen_budget_baseline.sh`:
#   default mode generates, `--check` is the CI guardrail, `--no-diff`
#   suppresses the git diff for CI bootstrap / scripting.
# - Uses `jq` for baseline parsing; the JSON shape matches
#   `contracts/.gas-baseline.json`.
# - Portable `stat` (GNU + BSD/macOS) — no Python dependency.
# - Hard-fails (exit 1) on missing JSON, missing WASM artifact, any
#   over-budget delta, or any uninitialized baseline in --check mode.
#
# Exit codes
# ----------
# 0  Success (regen wrote, or --check passed within tolerance).
# 1  Failure (any over-budget delta, missing baseline, missing WASM,
#    --check with uninitialized baseline).

set -euo pipefail

cd "$(dirname "$0")/.."

BASELINE="scripts/wasm-size-baseline.json"
MODE="regen"
SHOW_DIFF=true

for arg in "$@"; do
    case "$arg" in
        --regen)   MODE="regen"  ; SHOW_DIFF=true ;;
        --no-diff) MODE="regen"  ; SHOW_DIFF=false ;;
        --check)   MODE="check"  ; SHOW_DIFF=false ;;
        *) echo "Unknown argument: $arg" >&2
           echo "Expected: --regen | --no-diff | --check" >&2
           exit 1 ;;
    esac
done

# jq is required for both modes (used to parse + rewrite the JSON baseline).
if ! command -v jq >/dev/null 2>&1; then
    echo "::error::jq is required. Install with: apt-get install -y jq" >&2
    exit 1
fi

# Baseline must exist for both modes; --regen reads/writes, --check reads.
if [ ! -f "$BASELINE" ]; then
    echo "::error::Baseline file missing: $BASELINE" >&2
    echo "Seed it manually, then run: ./scripts/wasm-size-baseline.sh --regen" >&2
    exit 1
fi

TOLERANCE=$(jq -r '.tolerance_bytes // 5120' "$BASELINE")
echo "==> Tolerance: ±${TOLERANCE} bytes (±$((TOLERANCE / 1024)) KB)"

# Portable stat: GNU (-c %s) then BSD/macOS (-f %z).
# Returns 0 and prints the size in bytes, or 1 if size cannot be determined.
stat_size() {
    local f="$1"
    if [ ! -f "$f" ]; then
        return 1
    fi
    if stat -c "%s" "$f" >/dev/null 2>&1; then
        stat -c "%s" "$f"
    elif stat -f "%z" "$f" >/dev/null 2>&1; then
        stat -f "%z" "$f"
    else
        return 1
    fi
}

# Number of entries in `.crates`.
CRATE_COUNT=$(jq '.crates | length' "$BASELINE")

# ---------------------------------------------------------------- regen mode --
if [ "$MODE" = "regen" ]; then
    echo "==> Building workspace release WASM …"
    cargo build --release --target wasm32-unknown-unknown --workspace

    echo ""
    echo "==> Measuring WASM sizes …"

    # Single in-place scratch file reused across iterations (avoids per-iteration
    # mktemp leak under set -e). Cleanup on any exit code.
    tmp=""
    on_exit() {
        if [ -n "$tmp" ] && [ -f "$tmp" ]; then rm -f "$tmp"; fi
        if [ -n "${tmp_new:-}" ] && [ -f "$tmp_new" ]; then rm -f "$tmp_new"; fi
    }
    trap on_exit EXIT

    tmp="$(mktemp)"
    cp "$BASELINE" "$tmp"

    any_failed=0
    for (( i = 0; i < CRATE_COUNT; i++ )); do
        name=$(jq -r ".crates[$i].name" "$tmp")
        wasm_path=$(jq -r ".crates[$i].wasm_path" "$tmp")

        if ! size=$(stat_size "$wasm_path"); then
            echo "  [skip] ${name}: ${wasm_path} not found." >&2
            any_failed=1
            continue
        fi

        echo "  [ok]   ${name}: ${size} bytes"

        tmp_new="$(mktemp)"
        jq \
            --argjson idx "$i" \
            --argjson size "$size" \
            --arg stamp "regenerated by scripts/wasm-size-baseline.sh" \
            '.crates[$idx].size_bytes = $size
             | .crates[$idx].last_updated = $stamp' \
            "$tmp" > "$tmp_new"
        mv "$tmp_new" "$tmp"
        tmp_new=""
    done

    if [ "$any_failed" -ne 0 ]; then
        echo "" >&2
        echo "::error::One or more WASM artifacts are missing." >&2
        echo "         Did the workspace build succeed? Path resolution:" >&2
        jq -r '.crates[] | "  - \(.name): \(.wasm_path)"' "$BASELINE" >&2
        exit 1
    fi

    mv "$tmp" "$BASELINE"
    tmp=""

    echo ""
    echo "==> Baseline written to: $BASELINE"

    if [ "$SHOW_DIFF" = "true" ]; then
        if git diff --quiet -- "$BASELINE" 2>/dev/null; then
            echo "    No changes — baseline is up to date with current build."
        else
            echo ""
            echo "==> Diff (review before committing):"
            git diff -- "$BASELINE" || true
        fi
    fi

    echo ""
    echo "Done. If the numbers look correct, commit with:"
    echo "  git add $BASELINE"
    echo "  git commit -m 'chore: regen wasm size baseline'"
    exit 0
fi

# ---------------------------------------------------------------- check mode --
echo "==> Building workspace release WASM …"
cargo build --release --target wasm32-unknown-unknown --workspace

echo ""
echo "==> CI mode: enforcing ±${TOLERANCE} byte tolerance …"

printf '%-22s | %10s | %10s | %8s | %s\n' "crate" "baseline" "measured" "delta" "status"
printf -- '-%.0s' {1..72}; printf '\n'

fail_count=0
warn_count=0
notice_count=0
uninit_count=0

for (( i = 0; i < CRATE_COUNT; i++ )); do
    name=$(jq -r ".crates[$i].name" "$BASELINE")
    baseline=$(jq -r ".crates[$i].size_bytes" "$BASELINE")
    wasm_path=$(jq -r ".crates[$i].wasm_path" "$BASELINE")

    if ! measured=$(stat_size "$wasm_path"); then
        printf '%-22s | %10s | %10s | %8s | %s\n' "$name" "$baseline" "missing" "?" "ERROR"
        echo "::error::WASM artifact missing for ${name}: ${wasm_path}" >&2
        fail_count=$((fail_count + 1))
        continue
    fi

    if [ "${baseline:-0}" -eq 0 ]; then
        printf '%-22s | %10s | %10s | %8s | %s\n' "$name" "0" "$measured" "?" "UNINIT"
        echo "::error::Baseline for ${name} is uninitialized (size_bytes=0)." >&2
        echo "         Run scripts/wasm-size-baseline.sh --regen locally, then commit." >&2
        uninit_count=$((uninit_count + 1))
        continue
    fi

    delta=$(( measured - baseline ))

    # Robust bash arithmetic; tolerates negative literals across versions.
    if (( delta > tolerance )); then
        printf '%-22s | %10s | %10s | %+8d | %s\n' \
            "$name" "$baseline" "$measured" "$delta" "OVER"
        echo "::error::WASM size for ${name} grew by ${delta} bytes " \
             "(baseline=${baseline}, measured=${measured}, tolerance=±${TOLERANCE})" >&2
        fail_count=$((fail_count + 1))
    elif (( delta < -tolerance )); then
        abs_delta=$(( -delta ))
        printf '%-22s | %10s | %10s | %+8d | %s\n' \
            "$name" "$baseline" "$measured" "$delta" "UNDER"
        echo "::warning::WASM size for ${name} shrank by ${abs_delta} bytes " \
             "(baseline=${baseline}, measured=${measured}, tolerance=±${TOLERANCE})." >&2
        echo "         Verify no feature was unintentionally dropped." >&2
        warn_count=$((warn_count + 1))
    elif (( delta != 0 )); then
        printf '%-22s | %10s | %10s | %+8d | %s\n' \
            "$name" "$baseline" "$measured" "$delta" "OK"
        echo "::notice::WASM size drift for ${name}: ${delta} bytes (within ±${TOLERANCE}B budget)." >&2
        notice_count=$((notice_count + 1))
    else
        printf '%-22s | %10s | %10s | %+8d | %s\n' \
            "$name" "$baseline" "$measured" "$delta" "OK"
    fi
done

echo ""
if [ "$uninit_count" -gt 0 ]; then
    echo "FAILED: ${uninit_count} baseline(s) uninitialized. Run --regen and commit." >&2
    exit 1
elif [ "$fail_count" -gt 0 ]; then
    echo "FAILED: ${fail_count} over-budget, ${warn_count} under-budget warning(s), ${notice_count} within-budget notice(s)." >&2
    exit 1
elif [ "$warn_count" -gt 0 ] || [ "$notice_count" -gt 0 ]; then
    echo "PASSED: ${CRATE_COUNT} crate(s) within tolerance (${warn_count} warning(s), ${notice_count} notice(s))."
    exit 0
else
    echo "PASSED: all ${CRATE_COUNT} crate(s) within tolerance, no drift detected."
    exit 0
fi
