# Borrow Contract Error Catalog

This catalog documents the stable `ContractError` variants used by the borrow-facing credit contract. The numeric values and variant order are part of the contract ABI and are pinned by the discriminant tests in [contracts/credit/tests/error_discriminants.rs](../../contracts/credit/tests/error_discriminants.rs).

| Code | Variant | Description | Trigger Condition | Client Handling |
|------|----------|-------------|-------------------|-----------------|
| 1 | `Unauthorized` | Caller is not authorized to perform the requested action. | An entrypoint is invoked without the required authorization. | Reconnect with the correct wallet or role and retry. |
| 2 | `NotAdmin` | Caller does not have admin privileges for this entrypoint. | An admin-only operation is invoked by a non-admin. | Use an admin wallet or request admin access. |
| 3 | `CreditLineNotFound` | The requested borrower does not have an open credit line. | A borrower without a line attempts to draw, repay, or mutate state. | Create or open a credit line before retrying. |
| 4 | `CreditLineClosed` | The credit line is permanently closed and cannot accept new activity. | Draw or repay is attempted on a closed line. | Create a new line; closed lines are terminal. |
| 5 | `InvalidAmount` | The requested amount is zero, negative, or otherwise invalid for this operation. | A draw, repay, or other operation receives an invalid amount. | Validate amount input before retrying. |
| 6 | `OverLimit` | The requested draw would exceed the available credit line limit. | A draw would push utilization above the configured limit. | Reduce the draw amount to fit the available headroom. |
| 7 | `NegativeLimit` | The proposed credit limit cannot be negative. | A negative limit is passed to a credit-line configuration flow. | Use a non-negative limit value. |
| 8 | `RateTooHigh` | The requested rate exceeds the maximum allowed delta for the borrower. | An admin or risk update proposes a rate outside policy bounds. | Clamp the rate to the permitted range. |
| 9 | `ScoreTooHigh` | The supplied risk score is above the acceptable maximum threshold. | A credit line is opened or reconfigured with an out-of-range risk score. | Normalize the score to the supported range. |
| 10 | `UtilizationNotZero` | The requested transition requires zero outstanding utilization first. | A close or similar operation is attempted while debt remains outstanding. | Repay the outstanding balance before retrying. |
| 11 | `Reentrancy` | Reentrant execution was detected during a state-changing call. | A token hook or cross-contract path re-enters the contract. | Do not retry the same transaction; inspect the call path and state. |
| 12 | `Overflow` | An arithmetic operation overflowed or underflowed during contract logic. | A computation exceeds the supported integer range. | Retry with smaller amounts or a different configuration. |
| 13 | `LimitDecreaseRequiresRepayment` | Lowering the credit limit would leave outstanding debt above the new cap. | A limit decrease is attempted while utilization exceeds the new limit. | Repay debt until utilization is within the new limit. |
| 14 | `AlreadyInitialized` | The contract has already been initialized and cannot be initialized twice. | `init` is invoked more than once. | No further action is needed if the contract is already live. |
| 15 | `QuorumNotMet` | The oracle quorum requirement was not satisfied for the requested operation. | Oracle prices are submitted without enough agreeing feeds. | Submit more authoritative prices or wait for quorum. |
| 16 | `OracleNotFound` | The referenced oracle has not been registered in the configured registry. | An oracle operation references an unregistered oracle. | Register or configure the oracle before retrying. |
| 17 | `OracleAlreadyExists` | The referenced oracle is already registered and cannot be re-added. | An oracle is registered again with the same identifier. | Reuse the existing registration. |
| 15 | `AdminAcceptTooEarly` | Admin role acceptance was attempted before the required delay window elapsed. | `accept_admin` is called too soon after a pending admin change. | Wait for the full delay window before accepting. |
| 16 | `BorrowerBlocked` | The borrower is blocked from drawing credit. | A draw is attempted by a blocked borrower. | Resolve the block with the admin before retrying. |
| 17 | `DrawExceedsMaxAmount` | The requested draw exceeds the configured per-transaction maximum. | A draw exceeds the configured max draw amount. | Split the draw into smaller requests. |
| 18 | `Paused` | The protocol is currently paused and the requested action is blocked. | A state-changing operation is attempted while paused. | Retry after the protocol is unpaused. |
| 19 | `DrawsFrozen` | Global draws are temporarily frozen by admin for the protocol. | A draw is attempted while global freezes are active. | Wait for draws to be unfrozen or use a supported alternative flow. |
| 20 | `CreditLineSuspended` | The credit line is suspended and cannot accept the requested action. | A draw or state transition is attempted on a suspended line. | Wait for reinstatement or follow the cure path. |
| 21 | `CreditLineDefaulted` | The credit line is in default and requires cure or liquidation. | A draw or state transition is attempted on a defaulted line. | Repay to cure or proceed with the liquidation workflow. |
| 22 | `MissingLiquidityToken` | The liquidity token address has not been configured for the contract. | A draw or liquidity-dependent flow runs before the token is set. | Configure the liquidity token before retrying. |
| 23 | `MissingLiquiditySource` | The liquidity source address has not been configured for the contract. | A draw runs before a liquidity source is configured. | Configure the liquidity source before retrying. |
| 24 | `InsufficientLiquidityReserve` | The configured reserve balance cannot cover the requested draw. | Reserve balance is lower than the requested draw amount. | Wait for reserve replenishment or reduce the draw amount. |
| 25 | `LiquidityTokenCallFailed` | The liquidity token call failed in a way that the contract could observe. | The token transfer or transfer-from flow fails. | Retry after confirming the token contract and allowances. |
| 26 | `InsufficientRepaymentAllowance` | The borrower's token allowance is below the effective repayment amount. | Repayment is attempted without sufficient allowance. | Increase the allowance before retrying. |
| 27 | `InsufficientRepaymentBalance` | The borrower's token balance is below the effective repayment amount. | Repayment is attempted without sufficient balance. | Deposit or acquire more balance before retrying. |
| 28 | `RepayExceedsMaxAmount` | The repayment amount exceeds the configured per-transaction maximum. | A repay exceeds the configured max repay amount. | Split the repayment into smaller requests. |
| 29 | `DrawCooldownActive` | The borrower attempted to draw again before the cooldown interval elapsed. | Draws are attempted too quickly in succession. | Wait for the cooldown window to elapse. |
| 30 | `TreasuryNotSet` | The treasury address is not configured for the requested withdrawal flow. | A treasury withdrawal is proposed without a configured treasury. | Configure the treasury address. |
| 31 | `ExposureCapExceeded` | The requested draw would exceed the global protocol exposure cap. | A draw would exceed the configured protocol exposure cap. | Reduce the draw amount or wait for repayment. |
| 32 | `AdminNotInitialized` | The admin address has not been initialized in contract storage. | An admin-only action is invoked before initialization. | Call `init` with a valid admin address. |
| 33 | `TimestampRegression` | A timestamp write regressed relative to the previously stored value. | A state transition or update writes an out-of-order timestamp. | Re-sync the ledger view and retry the transaction. |
| 34 | `LimitOutOfBounds` | The proposed credit limit falls outside the configured min/max bounds. | A limit is configured outside the allowed range. | Adjust the proposed limit to the supported range. |
| 35 | `CollateralRatioBelowMinimum` | The collateral ratio is below the minimum required threshold. | A draw or collateral operation would violate the ratio policy. | Add collateral or reduce the operation size. |
| 36 | `OraclePriceInvalid` | The oracle price is zero, negative, or malformed and cannot be used. | An oracle feed returns an unusable price. | Use a valid price feed or retry after refresh. |
| 37 | `OraclePriceStale` | The oracle price is older than the configured freshness window. | A stale price is used for a settlement or policy check. | Wait for a fresh price update. |
| 38 | `OraclePriceDeviation` | The oracle price deviates beyond the configured threshold. | A price feed exceeds the deviation limit. | Wait for a new price within the acceptable range. |
| 39 | `InsufficientCollateralBalance` | The borrower's collateral balance is insufficient for the requested operation. | A collateral withdrawal exceeds the available balance. | Reduce the requested withdrawal or add more collateral. |
| 40 | `BorrowerFrozen` | The borrower is temporarily frozen from drawing until the expiry timestamp. | A draw is attempted while a per-borrower freeze is active. | Wait for the freeze to expire or contact the admin. |
| 41 | `BountyNotSet` | The bounty pool address is not configured for the requested withdrawal flow. | A bounty withdrawal is attempted without a configured bounty pool. | Configure the bounty address. |
| 42 | `NoPendingTreasuryWithdrawal` | No pending treasury withdrawal proposal exists for the requested execution. | A treasury withdrawal is executed without an existing proposal. | Create a proposal before executing. |
| 43 | `TreasuryTimelockActive` | The treasury withdrawal timelock has not elapsed yet. | A pending withdrawal is executed too early. | Wait for the timelock to elapse. |
| 44 | `TreasuryProposalExists` | A treasury withdrawal proposal already exists and must be resolved first. | A second proposal is submitted while a prior one is still pending. | Execute or cancel the existing proposal first. |
| 45 | `CloseFactorAboveMax` | The supplied close factor exceeds the protocol-configured maximum. | A liquidation or close flow uses an out-of-range close factor. | Reduce the close factor to the supported maximum. |
| 46 | `CreditLineFrozen` | The credit line is frozen by admin and cannot accept new draws. | A draw is attempted while the line is frozen. | Wait for the admin to unfreeze the line. |
| 47 | `DrawReversalWindowExpired` | The draw reversal window has expired and the reversal is no longer allowed. | A reversal is attempted after the allowed window. | No further reversal is possible. |
| 48 | `OriginalDrawNotFound` | The original draw record required for reversal was not found. | A reversal is attempted without a matching audit record. | No reversal is possible without the original draw record. |
| 49 | `AttestationBatchNotFound` | No attestation batch has been committed for the requested operation. | An attestation verification flow is invoked without a committed batch. | Commit or provide an existing attestation batch. |
| 50 | `OracleQuorumNotMet` | The oracle quorum condition was not satisfied by the available feeds. | Oracle price aggregation is attempted without enough feeds. | Wait for more price inputs or a different quorum configuration. |
| 51 | `AlreadySettled` | The liquidation settlement for this borrower and settlement identifier was already processed. | A settlement is replayed for the same borrower and settlement id. | No further action is required; the settlement is already complete. |
| 52 | `InvalidRiskWeight` | The collateral risk weight exceeds the maximum allowed value. | A risk weight outside the supported range is set. | Set the risk weight within the supported range. |
