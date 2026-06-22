# Integration and Initialization Guide

This guide turns the contract references into an operator-friendly runbook for
deploying Creditra, wiring the auction handoff, and recovering from common
initialization mistakes.

## Components

| Component | Path | Operator responsibility |
| --- | --- | --- |
| Credit contract | `contracts/credit/` | Owns credit-line lifecycle, liquidity configuration, risk parameters, collateral, accrual, default, and settlement. |
| Auction contract | `gateway-contract/contracts/auction_contract/` | Runs the liquidation auction and returns the winning bid through `settle_default_liquidation`. |
| Liquidity token | Stellar Asset Contract or compatible token | Must be configured before token-moving draw, repay, and collateral operations. |
| Liquidity source | Contract address or reserve account | Funds draw operations; defaults to the credit contract address after `init`. |
| Off-chain operator | Admin multisig or controlled operator account | Calls admin-only configuration, scorer updates, default, auction, and recovery actions. |
| Indexer | Off-chain event consumer | Reads Creditra and auction events for dashboards, monitoring, and reconciliation. |

## Pre-deploy checklist

- Build and test the workspace:

  ```bash
  cargo test --workspace
  cargo build --release --target wasm32-unknown-unknown -p creditra-credit
  cargo build --release --target wasm32-unknown-unknown -p gateway-auction
  ```

- Confirm the admin account is a multisig or controlled operator identity.
- Confirm the liquidity token contract address for the target network.
- Decide whether the reserve is the credit contract itself or an external reserve address.
- Choose initial operational bounds before opening borrower lines:
  `set_max_draw_amount`, `set_max_repay_amount`, `set_draw_min_interval`,
  `set_max_total_exposure`, and `set_credit_limit_bounds`.
- Decide whether oracle freshness/deviation enforcement is enabled through
  `set_oracle_config`.

## Deployment order

1. Deploy the credit WASM and record the contract id.
2. Deploy the auction WASM and record the contract id.
3. Initialize the credit contract with `init(admin)`.
4. Initialize or configure the auction contract according to the auction runbook.
5. Wire liquidity and auction dependencies on the credit contract:

   ```bash
   soroban contract invoke --id <credit-contract-id> --source <admin> --network <network> -- set_liquidity_token --token_address <token-contract>
   soroban contract invoke --id <credit-contract-id> --source <admin> --network <network> -- set_liquidity_source --reserve_address <reserve-address>
   soroban contract invoke --id <credit-contract-id> --source <admin> --network <network> -- set_auction_contract --auction_contract <auction-contract-id>
   ```

6. Apply operational policy:

   ```bash
   soroban contract invoke --id <credit-contract-id> --source <admin> --network <network> -- set_max_draw_amount --amount <i128>
   soroban contract invoke --id <credit-contract-id> --source <admin> --network <network> -- set_max_repay_amount --amount <i128>
   soroban contract invoke --id <credit-contract-id> --source <admin> --network <network> -- set_max_total_exposure --cap <i128>
   soroban contract invoke --id <credit-contract-id> --source <admin> --network <network> -- set_credit_limit_bounds --min <i128> --max <i128>
   soroban contract invoke --id <credit-contract-id> --source <admin> --network <network> -- set_rate_formula_config --base_bps <u32> --slope_bps <u32> --min_bps <u32> --max_bps <u32>
   ```

7. Run a small end-to-end rehearsal on testnet:
   `open_credit_line` -> `draw_credit` -> `repay_credit` -> default rehearsal
   with auction handoff -> `settle_default_liquidation`.

## Initialization semantics

`init(admin)` is intentionally one-shot. It writes the admin, schema version,
default liquidity source, and default collateral ratio. A second call reverts
with `AlreadyInitialized = 14`; do not try to recover by redeploying over the
same contract id. Deploy a new instance if the wrong admin was used.

`set_liquidity_token` is required before token-moving draw, repay, or collateral
operations. Without it, callers can hit `MissingLiquidityToken = 22`.

`set_liquidity_source` is optional when the credit contract itself is the reserve,
but explicit configuration is recommended for production so the operator and
indexer agree on the funding account.

`set_auction_contract` should be configured before any default-liquidation path
is enabled. Without it, default settlement orchestration cannot complete the
credit -> auction -> credit handoff.

## Operator setup

- Use a multisig admin for production. Keep single-key admins for local and
  throwaway testnet deployments only.
- Keep an operator log of every admin call, transaction hash, ledger, network,
  and expected event topic.
- Subscribe the indexer to credit events and auction events before opening the
  first live credit line.
- Treat scorer inputs as privileged data. Risk updates should be signed by the
  expected admin/operator flow, not submitted by borrowers.
- Keep `pause_protocol` and `freeze_draws` procedures rehearsed. Repayment must
  stay available during emergency draw freezes.

## Error recovery

| Symptom | Likely cause | Recovery |
| --- | --- | --- |
| `AlreadyInitialized = 14` on `init` | Contract was already initialized. | Verify the stored admin. If it is wrong, deploy a fresh instance. |
| `MissingLiquidityToken = 22` | `set_liquidity_token` was not called. | Configure the token contract, then retry the draw/repay/collateral operation. |
| `MissingLiquiditySource = 23` | Reserve address is absent where required. | Call `set_liquidity_source` or fund the default contract reserve path. |
| `InsufficientLiquidityReserve = 24` | Reserve cannot cover a draw. | Fund the reserve, lower exposure, or freeze draws while liquidity is restored. |
| `InsufficientRepaymentAllowance = 26` | Borrower has not approved enough token allowance. | Ask borrower to approve at least the effective repayment amount, including fees or penalties. |
| `Paused = 18` | Emergency pause is active. | Leave pause active until incident review is complete; use allowed repayment/recovery paths only. |
| `DrawsFrozen = 19` | Admin froze draws during reserve operations. | Complete reserve work, reconcile balances, then call `unfreeze_draws`. |
| Oracle rejection | Price is stale or outside configured deviation bounds. | Refresh the source price and verify `set_oracle_config` bounds before retry. |

## Post-deploy verification

- `get_contract_version` returns the expected release.
- Protocol config shows the intended liquidity token and source.
- The auction contract query confirms the auction handoff target.
- Event indexer sees a test credit-line event, draw event, repayment event, and
  auction settlement event.
- `cargo test --workspace` and release WASM build remain green for the deployed
  revision.

## Related references

- [`deploy.md`](./deploy.md) for the minimal deploy sequence.
- [`EXECUTION_QUALITY.md`](./EXECUTION_QUALITY.md) for testnet and mainnet checklists.
- [`PROTOCOL_SPEC.md`](./PROTOCOL_SPEC.md) for entrypoint signatures and validation order.
- [`contract-errors.md`](./contract-errors.md) for stable error discriminants.
- [`default-liquidation-auction-hook.md`](./default-liquidation-auction-hook.md)
  for the cross-contract settlement protocol.
- [`indexer-integration.md`](./indexer-integration.md) for event decoding.
