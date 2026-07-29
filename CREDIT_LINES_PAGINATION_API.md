# Credit Lines Pagination API Documentation

## Overview
This document describes the cursor-based pagination API for credit lines added to the Creditra credit contract as part of issue #781.

## New Type

### `CreditLinesPage`
A paginated view of credit lines for off-chain reporting and indexing.

**Location:** `contracts/credit/src/types.rs`

```rust
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditLinesPage {
    /// Vector of credit line data for this page.
    pub lines: soroban_sdk::Vec<CreditLineData>,
    /// Cursor for the next page, or `None` if this is the last page.
    pub next_cursor: Option<u32>,
    /// Whether more results are available beyond this page.
    pub has_more: bool,
}
```

## New Function

### `get_credit_lines_paginated`
Returns a paginated view of credit lines using cursor-based pagination.

**Location:** `contracts/credit/src/views.rs`

**Signature:**
```rust
pub fn get_credit_lines_paginated(env: Env, cursor: Option<u32>, limit: u32) -> CreditLinesPage
```

**Parameters:**
- `cursor`: Optional starting cursor (numeric ID). Pass `None` for the first page.
- `limit`: Maximum number of credit lines to return. Must be <= `MAX_ENUMERATION_LIMIT` (100).

**Returns:**
- `CreditLinesPage` containing:
  - `lines`: Vector of credit line data for this page.
  - `next_cursor`: Cursor for the next page, or `None` if this is the last page.
  - `has_more`: Boolean indicating if more results are available.

**Behavior:**
- Starts enumeration from `cursor.unwrap_or(0)`.
- Returns at most `limit` credit lines.
- Iterates through stable numeric IDs in ascending order.
- Skips IDs that have no corresponding borrower (gaps in the sequence).
- Bumps TTL for each credit line entry that is loaded.
- Enforces maximum limit to prevent unbounded gas consumption (panics with `ContractError::Overflow` if limit exceeds `MAX_ENUMERATION_LIMIT`).

**Example Usage:**
```text
// First page
let page1 = client.get_credit_lines_paginated(None, 10);

// Second page
if let Some(cursor) = page1.next_cursor {
    let page2 = client.get_credit_lines_paginated(Some(cursor), 10);
}
```

**Security:**
This is a read-only function with no authentication requirement. It only reads storage and does not mutate any state. The TTL bump on loaded entries is a side effect but does not change the logical state of the contract.

## Contract Entry Point

The function is exposed as a public contract method in `contracts/credit/src/lib.rs`:

```rust
pub fn get_credit_lines_paginated(env: Env, cursor: Option<u32>, limit: u32) -> CreditLinesPage {
    views::get_credit_lines_paginated(env, cursor, limit)
}
```

## Testing

Focused tests have been added in `contracts/credit/src/views_tests.rs`:

1. `test_credit_lines_pagination_first_page` - Tests retrieving the first page of results
2. `test_credit_lines_pagination_multiple_pages` - Tests navigating through multiple pages using cursors
3. `test_credit_lines_pagination_empty_result` - Tests behavior when no credit lines exist
4. `test_credit_lines_pagination_limit_enforcement` - Tests that limits exceeding MAX_ENUMERATION_LIMIT are rejected
5. `test_credit_lines_pagination_cursor_beyond_end` - Tests behavior when cursor is beyond the last credit line

## Storage Dependencies

The pagination function relies on the following storage helpers from `contracts/credit/src/storage.rs`:

- `get_credit_line_count()` - Returns the total number of indexed credit lines
- `get_borrower_by_credit_line_id(id)` - Returns the borrower address for a given numeric ID
- `get_credit_line(borrower)` - Returns the credit line data for a borrower

## Implementation Notes

- Uses stable numeric IDs assigned to each borrower via `ensure_credit_line_id()`
- Cursor-based pagination avoids offset-based limitations and is stateless
- Efficient for large datasets as it doesn't require skipping through previous results
- TTL bump on loaded entries ensures active borrower data remains accessible
