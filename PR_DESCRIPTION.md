# feat(settlement): cursor-based pagination for developer balances

Closes #428

## Summary

Adds `get_developer_balances_page(cursor, limit)` as a bounded, cursor-paginated
view over all borrower credit lines. Replaces the pattern of an unbounded
`get_all_developer_balances` full-scan: the backend can now retrieve all developer
balances one page at a time at predictable and bounded gas cost.

---

## Changes

### `contracts/credit/src/types.rs`

New `DeveloperBalance` `#[contracttype]` struct:

| Field | Type | Description |
|---|---|---|
| `id` | `u32` | Stable numeric cursor (insertion-order id) |
| `borrower` | `Address` | Borrower / developer address |
| `utilized_amount` | `i128` | Outstanding principal as of last mutation |
| `credit_limit` | `i128` | Configured credit ceiling |

### `contracts/credit/src/query.rs`

New `get_developer_balances_page(env, cursor, limit)`:

- `cursor` — exclusive lower bound on the stable id; `None` starts from the beginning
- `limit` — capped server-side at `MAX_ENUMERATION_LIMIT` (100)
- Returns `(Vec<DeveloperBalance>, Option<u32>)` where `next_cursor` is `Some(last_id)` when more pages exist, `None` at end of index
- O(limit) cost, independent of total credit-line count
- No auth required — pure read, safe for public RPC and off-chain indexers

### `contracts/credit/src/lib.rs`

- Public `Credit::get_developer_balances_page` entrypoint delegating to the query module
- New admin entrypoints: `set_min_collateral_ratio_bps` / `get_min_collateral_ratio_bps`
- `DeveloperBalance` added to the types import

### `contracts/credit/src/storage.rs`

Added missing storage helpers already referenced from `collateral.rs`, `config.rs`, and
`lib.rs` but not yet implemented:

- `get_collateral_balance` / `set_collateral_balance`
- `get_min_collateral_ratio_bps` / `set_min_collateral_ratio_bps`
- `get_collateral_token`
- `set_borrower_unblocked`

### `contracts/credit/tests/developer_balances_page.rs`

13 integration tests covering the full acceptance matrix:

| Test | What it checks |
|---|---|
| `empty_index_returns_empty_page_and_no_cursor` | `([], None)` on empty state |
| `single_entry_first_page` | id, borrower, utilized_amount, credit_limit all correct |
| `all_entries_fit_in_one_page_returns_no_cursor` | No cursor when results fit in one page |
| `cursor_advances_across_pages` | 3-page walk over 5 entries, cursor chain correct |
| `last_page_returns_none_cursor` | Exact-fit page returns `None` |
| `cursor_past_last_id_returns_empty` | Cursor beyond end returns `([], None)` |
| `limit_zero_returns_empty` | `limit=0` short-circuits cleanly |
| `limit_capped_at_max_enumeration_limit` | `limit=200` does not exceed internal cap |
| `stable_ordering_across_repeated_calls` | 3 identical calls return identical results |
| `cursor_unaffected_by_interleaved_credits` | New line opened mid-walk does not disrupt page 2 |
| `utilized_amount_is_zero_for_new_line` | Fresh line shows `utilized_amount = 0` |
| `utilized_amount_reflects_draw_via_enumerate` | Post-draw amount matches `enumerate_credit_lines` |
| `credit_limit_field_is_correct` | `credit_limit` correct for 3 distinct lines |
| `full_walk_collects_all_entries` | Loop walks all 7 entries in 3-per-page chunks, no gaps |

---

## Cursor stability guarantees

- **New credit lines** opened between pages appear at ids higher than any already-returned
  id — a sequential walk never misses entries.
- **Interleaved draws / repays** do not affect id ordering, so a mid-walk cursor stays valid.
- **Limit** is capped server-side at 100; clients cannot exceed it.

---

## Usage

```rust
// Walk all developer balances, 20 per page
let mut cursor: Option<u32> = None;
loop {
    let (page, next) = client.get_developer_balances_page(&cursor, &20);
    for entry in page.iter() {
        // entry.id, entry.borrower, entry.utilized_amount, entry.credit_limit
    }
    match next {
        Some(c) => cursor = Some(c),
        None => break,
    }
}
```

---

## Testing

```bash
cargo test -p creditra-credit developer_balances_page
```
