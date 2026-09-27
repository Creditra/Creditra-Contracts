| Code | Variant | Category | Trigger |
|:---:|---|---|---|
| 1 | `Unauthorized` | Auth | Caller is not authorized for this action |
| 2 | `NotAdmin` | Auth | Caller does not have admin privileges |
| 3 | `CreditLineNotFound` | Misc | Credit line does not exist |
| 4 | `CreditLineClosed` | Lifecycle | Credit line is permanently closed |
| 5 | `InvalidAmount` | Numeric | Amount is zero, negative, or otherwise invalid |
| 6 | `OverLimit` | Limit | Draw would exceed the credit limit |
| 7 | `NegativeLimit` | Numeric | Credit limit cannot be negative |
| 8 | `RateTooHigh` | Risk | Interest rate exceeds the maximum allowed |
| 9 | `ScoreTooHigh` | Risk | Risk score exceeds the maximum allowed (100) |
| 10 | `UtilizationNotZero` | Limit | Operation requires zero utilization |
| 11 | `Reentrancy` | Reentrancy | Reentrancy detected during cross-contract call |
| 12 | `Overflow` | Numeric | Arithmetic overflow during calculation |
| 13 | *(Unused)* | - | - |
| 14 | `AlreadyInitialized` | Lifecycle | Contract already initialized |
| 15 | `AdminAcceptTooEarly` | Misc | Admin acceptance attempted before delay elapsed |
| 16 | `BorrowerBlocked` | Block | Borrower is on the blocked list |
| 17 | `DrawExceedsMaxAmount` | Limit | Draw amount exceeds per-transaction cap |
| 18 | `Paused` | Risk | Protocol is paused; operation blocked by circuit breaker |
| 19 | `DrawsFrozen` | Block | Draws are globally frozen |
| 20 | `CreditLineSuspended` | Lifecycle | Credit line is suspended |
| 21 | `CreditLineDefaulted` | Lifecycle | Credit line is defaulted |
| 22 | `MissingLiquidityToken` | Liquidity | Liquidity token is not configured |
| 23 | `MissingLiquiditySource` | Liquidity | Liquidity source is not configured |
| 24 | `InsufficientLiquidityReserve` | Liquidity | Reserve balance cannot cover the draw |
| 25 | `LiquidityTokenCallFailed` | Liquidity | Liquidity token transfer call failed (observable error path). |
| 26 | `InsufficientRepaymentAllowance` | Liquidity | Borrower repayment allowance is insufficient to cover the repayment amount. |
| 27 | `InsufficientRepaymentBalance` | Liquidity | Borrower balance is insufficient to cover the repayment amount. |
| 28 | `RepayExceedsMaxAmount` | Limit | Repay amount exceeds per-transaction cap |
| 29 | `DrawCooldownActive` | Risk | Borrower attempted to draw before cooldown elapsed |
| 30 | `TreasuryNotSet` | Liquidity | Treasury address is not configured |
| 31 | `ExposureCapExceeded` | Liquidity | Draw would exceed the global protocol exposure cap |
| 32 | `AdminNotInitialized` | Auth | Admin address has not been initialized |
| 33 | `TimestampRegression` | Numeric | Timestamp regression detected |
| 34 | `LimitOutOfBounds` | Numeric | Credit limit is outside configured min/max bounds |
| 35 | `CollateralRatioBelowMinimum` | Collateral | Collateral ratio is below the minimum required ratio |
| 36 | `OraclePriceInvalid` | Oracle | Oracle price is invalid (zero, negative, or malformed) |
| 37 | `OraclePriceStale` | Oracle | Oracle price is stale (exceeds max_age_seconds) |
| 38 | `OraclePriceDeviation` | Oracle | Oracle price deviation exceeds the configured maximum |
| 39 | `InsufficientCollateralBalance` | Collateral | Borrower collateral balance cannot cover withdrawal |
| 40 | `BorrowerFrozen` | Block | Borrower's draws are temporarily frozen until expiry |
| 41 | `BountyNotSet` | Liquidity | Bounty pool address is not configured |
| 42 | `NoPendingTreasuryWithdrawal` | Misc | No pending treasury withdrawal proposal exists |
| 43 | `TreasuryTimelockActive` | Misc | Treasury withdrawal timelock has not yet elapsed since proposal. |
| 44 | `TreasuryProposalExists` | Misc | A treasury withdrawal proposal already exists; cancel or execute it first. |
| 45 | `CloseFactorAboveMax` | Limit | The supplied `close_factor_bps` exceeds the protocol-configured maximum. |
| 46 | `CreditLineFrozen` | Block | Credit line draws are frozen by admin (compliance hold) |
| 47 | `DrawReversalWindowExpired` | Limit | Draw reversal attempted after the allowed window expired |
| 48 | `OriginalDrawNotFound` | Misc | Original draw record not found for reversal |
| 49 | `AttestationBatchNotFound` | Misc | No attestation batch has been committed |
| 50 | `OracleQuorumNotMet` | Oracle | Oracle quorum condition not satisfied |
| 51 | `AlreadySettled` | Lifecycle | Liquidation settlement already processed for this (borrower, id) pair |
| 52 | `InvalidRiskWeight` | Numeric | Collateral risk weight exceeds the maximum allowed (10 000 bps) |
| 53 | `InvalidAttestation` | Misc | Attestation proof is invalid or no attestation batch has been committed |
| 54 | `RiskAdminCooldownActive` | Risk | Risk admin cooldown has not yet elapsed since the last mutation |
| 55 | `OracleNotFound` | Oracle | Oracle address not in the registry |
| 56 | *(Unused)* | - | - |
| 57 | `FreezeCooldownActive` | Block | Freeze cooldown active |
| 58 | `AdminCollateralCooldownActive` | Collateral | Admin collateral cooldown active |
| 59 | `LiquidationGraceActive` | Lifecycle | Per-borrower liquidation grace window active |
| 60 | `StaleStateTransition` | Lifecycle | Transition rejected because the credit line is already in the requested target state (stale/duplicate). |
| 61 | `IncompatibleVersion` | Handshake | Auction contract's protocol version does not match the credit contract. The version handshake check failed before any state mutation occurred. The reentrancy guard has been cleared; the settlement is safe to retry once the auction or credit contract is upgraded to a compatible version. |
| 62 | `AuctionCallFailed` | Handshake | The cross-contract auction CPI call failed or returned an unexpected value. No credit-line state was mutated. The reentrancy guard has been cleared. The settlement is safe to retry with a corrected `recovered_amount` or after the auction contract issue is resolved. |
| 63 | `AuctionActive` | Lifecycle | A fee-configuration change was rejected because at least one liquidation auction is currently active (Issue #1169). Fee parameters — protocol fee, treasury/bounty fee-share split, penalty surcharge, and flat / structured late fees — are frozen while any defaulted credit line has an in-flight liquidation auction, so that the economics of an ongoing auction and its eventual settlement are deterministic. The block lifts when the last active auction exits the `Defaulted` pipeline (full settlement, reinstate, force-close, or reopen). |
