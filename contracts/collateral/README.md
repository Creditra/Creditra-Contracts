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

Otherwise the contract reverts with `ContractError::AdminCollateralCooldownActive` (`56`).

Implementation: [`src/admin.rs`](./src/admin.rs) (compiled into `creditra-credit`).
