# Moved: `ContractError` codes

This file is now a redirect stub. The canonical error reference is
**[`docs/errors.md`](./errors.md)**.

`docs/ERROR_CODES.md` previously held a categorized copy of the credit
contract's `ContractError` table. That table is now merged into the canonical
page, which publishes each code once, alongside its `ContractErrorCategory`,
its trigger condition, and its SDK recovery action.

The source of truth for the codes is
[`ContractError`](../contracts/credit/src/types.rs). Do not re-add a table here.
