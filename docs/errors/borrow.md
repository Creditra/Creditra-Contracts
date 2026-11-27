# Moved: borrow-facing `ContractError` catalog

This file is now a redirect stub. The canonical error reference is
**[`docs/errors.md`](../errors.md)**.

The borrow path does not define its own error enum — `draw_credit` and
`repay_credit` live on the credit contract and raise `ContractError` from
[`contracts/credit/src/types.rs`](../../contracts/credit/src/types.rs). The
catalog that used to live here was a stale copy of that table and is removed.

The source of truth is
[`ContractError`](../../contracts/credit/src/types.rs). Do not re-add a table
here.
