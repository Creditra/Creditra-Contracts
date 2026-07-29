# Borrow error stability (v7)

CI guard that freezes client-facing `ContractError` discriminants for the
borrow / draw / repay surface (`draw_credit`, `repay_credit`,
`repay_and_release_collateral`, `reverse_draw`, and related gates).

## Run

```bash
cargo test -p creditra-borrow --test err_stab
```

## See also

- [`contracts/credit/src/borrow.rs`](../credit/src/borrow.rs)
- [`contracts/credit/src/types.rs`](../credit/src/types.rs) — `ContractError`
- [`contracts/credit/tests/error_discriminants.rs`](../credit/tests/error_discriminants.rs)
