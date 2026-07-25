# Collateral admin cool-off (v7)

Critical collateral admin entrypoints on the credit contract enforce a shared
cool-off interval between mutations:

- `set_min_collateral_ratio_bps`
- `set_collateral_risk_weight`
- `set_collateral_token_allowlist`

## Configuration

| Entrypoint | Storage key | Notes |
| --- | --- | --- |
| `set_admin_collateral_cooldown_seconds(seconds)` | `AdminCollateralCooldownSeconds` | Admin only. `seconds = 0` disables the guard. Does **not** consume the cool-off clock. |
| `get_admin_collateral_cooldown_seconds()` | — | Returns `Option<u64>`. |
| `get_last_admin_collateral_critical_action_ts()` | `LastAdminCollateralCriticalActionTs` | Ledger timestamp of the last successful critical action. |

## Enforcement

When `AdminCollateralCooldownSeconds` is set to a positive value, each critical
action requires:

```text
ledger.timestamp >= LastAdminCollateralCriticalActionTs + AdminCollateralCooldownSeconds
```

Otherwise the contract reverts with `ContractError::AdminCollateralCooldownActive` (`54`).

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
