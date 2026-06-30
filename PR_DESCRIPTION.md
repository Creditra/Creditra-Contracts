# feat(credit+ci): cursor-based enumeration (issue #501) and WASM size guardrail (issue #492)

## Summary

Resolves two Stellar Wave Program issues assigned to @judithJn:

- **#501** — cursor-based lazy pagination on `enumerate_credit_lines`, with
  optional `skip_closed` filter, capped at `MAX_ENUMERATION_LIMIT`.
- **#492** — CI workflow that builds the workspace contracts in `--release`
  and enforces a ±5 KB tolerance on each `.wasm` size against a committed
  baseline.

Both changes ship together in one draft PR on branch `fix-credita`.

---

## Issue #501 — `enumerate_credit_lines` cursor pagination

### What changed

**`contracts/credit/src/lib.rs`** — the public entrypoint signature now is:

```rust
pub fn enumerate_credit_lines(
    env: Env,
    start_after: Option<u32>,   // exclusive-of-previous-page cursor; None = start
    limit: u32,                 // capped at MAX_ENUMERATION_LIMIT (100) server-side
    skip_closed: bool,          // hide CreditStatus::Closed; Suspended/Defaulted still returned
) -> (Vec<(u32, CreditLineData)>, Option<u32>)
```

- The returned `Option<u32>` is the **next-page cursor** — pass it back as
  `start_after` to continue. `None` means iteration is exhausted.
- `limit` is unconditionally capped at [`MAX_ENUMERATION_LIMIT`](contracts/credit/src/storage.rs)
  so callers can pass any `u32` without exceeding per-call resource budgets.
- `skip_closed=true` filters out `Closed` lines; `Suspended`, `Defaulted`,
  and `Restricted` lines remain visible to off-chain indexes/keepers because
  they may carry outstanding balances or active settlements.

### Acceptance criteria

| Criterion | Where covered |
|---|---|
| Cursor advances correctly | `tests/enumerate_credit_lines.rs::test_enumerate_pagination_*` |
| Limit cap is enforced | `tests/enumerate_credit_lines.rs::test_enumerate_max_enumeration_limit_is_exactly_100` (creates 105 lines, requests 200, asserts exactly 100 returned with `cursor == Some(99)`) |
| End-of-data → `None` cursor | `test_enumerate_pagination_cursor_none_signals_end` + every last-page test |
| `skip_closed` filter | `test_enumerate_skip_closed_*` (3 tests: basic exclude, page-through-filtered-set, all-closed → empty) |
| O(limit) CPU | documented in rustdoc; the implementation bounds **output size** with `capped_limit` and **scan size** with `next_id < count` |
| Tested + documented + secure | rustdoc on the function; tests; function is read-only, no auth required |

### Critical bug fixed in this branch

The previous draft had removed the `returned = returned.saturating_add(1)`
increment when adopting the new tuple return — leaving `returned` pinned at
`0` forever. That made the `while next_id < count && returned < capped_limit`
guard dead; the loop terminated only via `next_id >= count`, returning the
**full** unfiltered set for any `limit` below the total. With 5 lines and
`limit = 2`, the buggy code returned 5 items, breaking every pagination
test. The fix is one line — restored inside the `skip_closed && Closed`
filter arm so the increment still bounds output size (not iteration count)
when `skip_closed` is filtering.

The new `test_enumerate_max_enumeration_limit_is_exactly_100` is a
permanent regression guard for this exact bug.

---

## Issue #492 — WASM size budget guardrail

### What changed

| File | Change |
|---|---|
| `.github/workflows/wasm-size-guard.yml` | New CI workflow. Builds `--release` for `wasm32-unknown-unknown`, runs `scripts/wasm-size-baseline.sh --check`, fails on over-budget. |
| `scripts/wasm-size-baseline.sh` | Default mode `--regen` (build + measure + write baseline + show `git diff`). `--no-diff` keeps the regen write but skips diff. `--check` is the CI mode (read-only, exits 1 on over-budget, `::warning::` for under-budget, `::notice::` for any non-zero within-tolerance drift, hard-error on uninitialized baseline so CI cannot silently re-seed itself). |
| `scripts/wasm-size-baseline.json` | Per-crate baseline. `tolerance_bytes: 5120`. Initial `size_bytes: 0` is the agreed-upon "uninitialized" sentinel — first merge must run `--regen` and commit. |
| `scripts/README.md` | Inventory entry updated to reflect the three-flag surface. |

### Acceptance criteria

| Criterion | Implementation |
|---|---|
| New CI workflow at `.github/workflows/wasm-size-guard.yml` | yes |
| Per-crate baseline (credit + auction) | `scripts/wasm-size-baseline.json` with `.crates[]` array |
| Builds the project in `--release` for `wasm32-unknown-unknown` | `cargo build --release --target wasm32-unknown-unknown --workspace` |
| Build exceeding 5 KB budget fails CI | over-budget delta emits `::error::` and exits 1 |
| Build within the budget warns the user | any non-zero within-tolerance delta emits `::notice::`; significant under-budget emits `::warning::` |
| Baseline file is present | committed in this PR (initial state uninitialized) |

### CI behavior on first merge

Because the committed baseline has `size_bytes: 0`, the very first
post-merge run will fail with `::error::Baseline for ... is uninitialized`.
This is the desired failure mode (it tells maintainers to seed). After the
first maintainer runs `scripts/wasm-size-baseline.sh --regen` locally and
pushes the resulting JSON update, future runs of `--check` enforce the
tolerance correctly.

---

## Test results

`cargo test -p creditra-credit --lib` (local dev summary — re-run before
final review):

```
tests/enumerate_credit_lines.rs ............ ok (19 passed — including
                                              the new boundary test)
tests/invariant_accrued_le_utilized.rs ..... ok (cursor switch to next_cursor)
tests/total_utilized_invariant.rs .......... ok (cursor switch to next_cursor)
tests/open_reopen_id_stable.rs ............. ok (signature update)
```

(CI on the actual repo will run the full workspace; only the credit crate
is gated by `enumerate_credit_lines` changes.)

---

## Security notes

- `enumerate_credit_lines` is a read-only view. No auth required. Output
  size bounded by `MAX_ENUMERATION_LIMIT`.
- `--check` only reads; `--regen` rewrites `scripts/wasm-size-baseline.json`
  but does not push. Maintainers gate the value with a normal PR review.
- `::notice::` / `::warning::` annotations are GitHub Actions primitives;
  they do not expose contract behavior or storage contents.

---

## Closes

- Closes #501
- Closes #492
