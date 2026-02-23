# Admin Access Control Security Note

## Admin Powers

The credit contract stores a single admin in instance storage (`DataKey::Admin`).
Only this admin can authorize sensitive operations:

- `update_risk_parameters`
- `suspend_credit_line`
- `default_credit_line`
- forced close via `close_credit_line` when `closer == admin`

Non-admin attempts fail due authorization checks or `ContractError::Unauthorized`.

## Initialization Invariant

`init` is one-time and guarded by `ContractError::AlreadyInitialized`.
After the first successful initialization, subsequent `init` calls cannot replace the admin.
The stored admin remains unchanged.

## Upgrade / Admin Rotation

This contract currently does not expose an on-chain admin-rotation function (`set_admin` / `transfer_admin`)
and does not implement an explicit in-contract upgrade manager in `contracts/credit`.
Admin rotation or code upgrade must therefore be handled through deployment/governance processes external to this contract.

## Operational Recommendations

- Use a multisig or threshold-controlled admin account.
- Keep admin keys in hardware-backed custody and rotate keys through governance-approved redeploy processes.
- Restrict who can initiate admin transactions and monitor admin activity on-chain.
