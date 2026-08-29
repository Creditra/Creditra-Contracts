# Collateral (v7)

## Error stability

CI guard that freezes client-facing `CollateralError` discriminants (mirror
tier + `100+` collateral-specific tier) for SDK / indexer ABI stability.

```bash
cargo test -p creditra-collateral --test err_stab
cargo test -p creditra-collateral --test catalog
```

See [`tests/err_stab.rs`](./tests/err_stab.rs) and
[`docs/errors/collateral.md`](../../docs/errors/collateral.md).

## Collateral admin cool-off (v7)

Critical collateral admin entrypoints on the credit contract enforce a shared
cool-off interval between mutations:

- `set_min_collateral_ratio_bps`
- `set_collateral_risk_weight`
- `set_collateral_token_allowlist`

## Configuration

| Entrypoint | Storage key | Notes |
| --- | --- | --- |
| `set_col_admin_cooldown_secs(seconds)` | `AdminCollateralCooldownSeconds` | Admin only. `seconds = 0` disables the guard. Does **not** consume the cool-off clock. |
| `get_col_admin_cooldown_secs()` | — | Returns `Option<u64>`. |
| `get_last_col_admin_action_ts()` | `LastColAdminActionTs` | Ledger timestamp of the last successful critical action. |

## Enforcement

When `AdminCollateralCooldownSeconds` is set to a positive value, each critical
action requires:

```text
ledger.timestamp >= LastColAdminActionTs + AdminCollateralCooldownSeconds
```

Otherwise the contract reverts with `ContractError::AdminCollateralCooldownActive` (`58`).

Implementation: [`src/admin.rs`](./src/admin.rs) (compiled into `creditra-credit`).

## Authorization snapshot

The collateral v7 API has per-entrypoint authorization regression coverage in
[`tests/auth_snap.rs`](./tests/auth_snap.rs). Each state-changing collateral
entrypoint records exactly one required signer:

- borrower authorization for deposit, withdrawal, partial release, atomic
  repay-and-release, and per-token deposit/withdrawal;
- admin authorization for collateral ratio, risk weight, token allowlist, and
  admin cooldown configuration.

Collateral queries remain authorization-free. This documents and tests the
existing public authorization contract; no function signatures changed.
