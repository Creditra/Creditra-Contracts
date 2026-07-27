//! Per-market protocol fee split between treasury and bounty pools.
//!
//! When a borrower repays a draw, the protocol fee is split between two
//! accumulators based on a configurable ratio:
//!
//! - **Treasury** — withdrawable by the contract owner via `WithdrawTreasury`.
//! - **Bounty pool** — withdrawable by the contract owner via `WithdrawBounty`.
//!
//! Each market (identified by its credit denomination) may have its own fee
//! split ratio. When no per-market override is set, the default ratio applies.
//!
//! The treasury share is computed with floor rounding; the bounty pool receives
//! the remainder so no tokens are lost to integer division.

use cosmwasm_std::{Deps, DepsMut, Uint128};
use cw_storage_plus::Item;

use crate::error::ContractError;
use crate::state::{
    Config, BOUNTY_BALANCE, CONFIG, DEFAULT_FEE_SHARE_BPS, MARKET_FEE_SHARE_BPS, TREASURY_BALANCE,
};

/// Maximum protocol fee rate chargeable on a repayment (10% == 1_000 bps).
pub const MAX_PROTOCOL_FEE_BPS: u32 = 1_000;

/// Total protocol fee rate (in basis points) skimmed from each repayment.
///
/// Distinct from [`DEFAULT_FEE_SHARE_BPS`]/[`MARKET_FEE_SHARE_BPS`], which
/// only govern how this fee is split between the treasury and bounty pools.
pub const PROTOCOL_FEE_BPS: Item<u32> = Item::new("pf_bps");

/// Maximum basis points for a fee-share ratio (100%).
pub const MAX_FEE_SHARE_BPS: u32 = 10_000;

/// Default treasury share when unset: 100% to treasury (backward compatible).
pub const DEFAULT_TREASURY_FEE_SHARE_BPS: u32 = 10_000;

/// Result of splitting a protocol fee between treasury and bounty accumulators.
#[derive(Clone, Debug, PartialEq)]
pub struct FeeSplitAmounts {
    /// Portion credited to the treasury balance for the market.
    pub treasury_amount: Uint128,
    /// Portion credited to the bounty pool balance for the market.
    pub bounty_amount: Uint128,
}

/// Split `total_fee` by `treasury_share_bps` in the range `0..=10_000`.
///
/// Treasury receives `floor(total_fee * treasury_share_bps / 10_000)`; the
/// bounty pool receives the remainder.
///
/// # Errors
///
/// Returns [`ContractError::InvalidFeeShareBps`] if `treasury_share_bps`
/// exceeds [`MAX_FEE_SHARE_BPS`].
///
/// # Examples
///
/// ```
/// // 50/50 split
/// let split = split_protocol_fee(Uint128::new(100), 5_000).unwrap();
/// assert_eq!(split.treasury_amount, Uint128::new(50));
/// assert_eq!(split.bounty_amount, Uint128::new(50));
/// ```
pub fn split_protocol_fee(
    total_fee: Uint128,
    treasury_share_bps: u32,
) -> Result<FeeSplitAmounts, ContractError> {
    if treasury_share_bps > MAX_FEE_SHARE_BPS {
        return Err(ContractError::InvalidFeeShareBps);
    }

    if total_fee.is_zero() {
        return Ok(FeeSplitAmounts {
            treasury_amount: Uint128::zero(),
            bounty_amount: Uint128::zero(),
        });
    }

    if treasury_share_bps == 0 {
        return Ok(FeeSplitAmounts {
            treasury_amount: Uint128::zero(),
            bounty_amount: total_fee,
        });
    }

    if treasury_share_bps >= MAX_FEE_SHARE_BPS {
        return Ok(FeeSplitAmounts {
            treasury_amount: total_fee,
            bounty_amount: Uint128::zero(),
        });
    }

    let treasury_amount = total_fee
        .checked_mul(Uint128::from(treasury_share_bps))
        .map_err(|_| ContractError::Std(cosmwasm_std::StdError::generic_err("Fee split overflow")))?
        .checked_div(Uint128::from(MAX_FEE_SHARE_BPS))
        .map_err(|_| {
            ContractError::Std(cosmwasm_std::StdError::generic_err(
                "Fee split division by zero",
            ))
        })?;
    let bounty_amount = total_fee.checked_sub(treasury_amount).map_err(|_| {
        ContractError::Std(cosmwasm_std::StdError::generic_err(
            "Fee split subtraction overflow",
        ))
    })?;

    Ok(FeeSplitAmounts {
        treasury_amount,
        bounty_amount,
    })
}

/// Return the effective treasury fee share in basis points for a given market.
///
/// Checks the per-market override first, then falls back to the default.
/// Returns [`DEFAULT_TREASURY_FEE_SHARE_BPS`] (10_000) if neither is set.
pub fn get_treasury_fee_share_bps(deps: Deps, market_denom: &str) -> u32 {
    MARKET_FEE_SHARE_BPS
        .may_load(deps.storage, market_denom)
        .unwrap_or(None)
        .or_else(|| DEFAULT_FEE_SHARE_BPS.load(deps.storage).ok())
        .unwrap_or(DEFAULT_TREASURY_FEE_SHARE_BPS)
}

/// Credit a fee to the treasury and bounty accumulators for a given market.
///
/// Splits `total_fee` using the per-market (or default) treasury share ratio
/// and credits the respective balances. Returns the [`FeeSplitAmounts`] for
/// event emission by the caller.
///
/// # Errors
///
/// Propagates storage errors and fee split validation errors.
pub fn accrue_protocol_fee(
    deps: &mut DepsMut,
    market_denom: &str,
    total_fee: Uint128,
) -> Result<FeeSplitAmounts, ContractError> {
    if total_fee.is_zero() {
        return Ok(FeeSplitAmounts {
            treasury_amount: Uint128::zero(),
            bounty_amount: Uint128::zero(),
        });
    }

    let treasury_share_bps = get_treasury_fee_share_bps(deps.as_ref(), market_denom);
    let split = split_protocol_fee(total_fee, treasury_share_bps)?;

    if !split.treasury_amount.is_zero() {
        let current = TREASURY_BALANCE
            .may_load(deps.storage, market_denom)?
            .unwrap_or_else(Uint128::zero);
        let updated = current.checked_add(split.treasury_amount).map_err(|_| {
            ContractError::Std(cosmwasm_std::StdError::generic_err(
                "Treasury balance overflow",
            ))
        })?;
        TREASURY_BALANCE.save(deps.storage, market_denom, &updated)?;
    }

    if !split.bounty_amount.is_zero() {
        let current = BOUNTY_BALANCE
            .may_load(deps.storage, market_denom)?
            .unwrap_or_else(Uint128::zero);
        let updated = current.checked_add(split.bounty_amount).map_err(|_| {
            ContractError::Std(cosmwasm_std::StdError::generic_err(
                "Bounty balance overflow",
            ))
        })?;
        BOUNTY_BALANCE.save(deps.storage, market_denom, &updated)?;
    }

    Ok(split)
}

/// Validate the owner is the caller. Returns the loaded config.
pub fn assert_owner(deps: Deps, sender: &cosmwasm_std::Addr) -> Result<Config, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if sender != config.owner {
        return Err(ContractError::Unauthorized);
    }
    Ok(config)
}

/// Withdraw treasury funds for a given market denomination.
///
/// Debits `amount` from the treasury balance. Caller is responsible for
/// sending the funds to the treasury address. Only callable by the contract
/// owner.
///
/// # Errors
///
/// Returns [`ContractError::InsufficientTreasuryBalance`] if the balance
/// is less than the requested amount.
pub fn withdraw_treasury(
    deps: &mut DepsMut,
    sender: &cosmwasm_std::Addr,
    market_denom: &str,
    amount: Uint128,
) -> Result<Uint128, ContractError> {
    assert_owner(deps.as_ref(), sender)?;

    if amount.is_zero() {
        return Err(ContractError::InvalidAmount);
    }

    let balance = TREASURY_BALANCE
        .may_load(deps.storage, market_denom)?
        .unwrap_or_else(Uint128::zero);

    if amount > balance {
        return Err(ContractError::InsufficientTreasuryBalance);
    }

    let updated = balance.checked_sub(amount).map_err(|_| {
        ContractError::Std(cosmwasm_std::StdError::generic_err(
            "Treasury withdrawal underflow",
        ))
    })?;
    TREASURY_BALANCE.save(deps.storage, market_denom, &updated)?;

    Ok(amount)
}

/// Withdraw bounty funds for a given market denomination.
///
/// Debits `amount` from the bounty balance. Caller is responsible for
/// sending the funds to the bounty address. Only callable by the contract
/// owner.
///
/// # Errors
///
/// Returns [`ContractError::InsufficientBountyBalance`] if the balance
/// is less than the requested amount.
pub fn withdraw_bounty(
    deps: &mut DepsMut,
    sender: &cosmwasm_std::Addr,
    market_denom: &str,
    amount: Uint128,
) -> Result<Uint128, ContractError> {
    assert_owner(deps.as_ref(), sender)?;

    if amount.is_zero() {
        return Err(ContractError::InvalidAmount);
    }

    let balance = BOUNTY_BALANCE
        .may_load(deps.storage, market_denom)?
        .unwrap_or_else(Uint128::zero);

    if amount > balance {
        return Err(ContractError::InsufficientBountyBalance);
    }

    let updated = balance.checked_sub(amount).map_err(|_| {
        ContractError::Std(cosmwasm_std::StdError::generic_err(
            "Bounty withdrawal underflow",
        ))
    })?;
    BOUNTY_BALANCE.save(deps.storage, market_denom, &updated)?;

    Ok(amount)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::mock_dependencies;

    // ── split_protocol_fee unit tests ───────────────────────────────────

    #[test]
    fn split_all_to_treasury_when_share_is_max() {
        let split = split_protocol_fee(Uint128::new(100), MAX_FEE_SHARE_BPS).unwrap();
        assert_eq!(
            split,
            FeeSplitAmounts {
                treasury_amount: Uint128::new(100),
                bounty_amount: Uint128::zero(),
            }
        );
    }

    #[test]
    fn split_all_to_bounty_when_share_is_zero() {
        let split = split_protocol_fee(Uint128::new(100), 0).unwrap();
        assert_eq!(
            split,
            FeeSplitAmounts {
                treasury_amount: Uint128::zero(),
                bounty_amount: Uint128::new(100),
            }
        );
    }

    #[test]
    fn split_even_ratio_allocates_half_each() {
        let split = split_protocol_fee(Uint128::new(100), 5_000).unwrap();
        assert_eq!(
            split,
            FeeSplitAmounts {
                treasury_amount: Uint128::new(50),
                bounty_amount: Uint128::new(50),
            }
        );
    }

    #[test]
    fn split_remainder_goes_to_bounty_on_rounding() {
        let split = split_protocol_fee(Uint128::new(10), 3_333).unwrap();
        assert_eq!(split.treasury_amount, Uint128::new(3));
        assert_eq!(split.bounty_amount, Uint128::new(7));
        assert_eq!(
            split.treasury_amount + split.bounty_amount,
            Uint128::new(10)
        );
    }

    #[test]
    fn split_zero_fee_yields_zeroes() {
        let split = split_protocol_fee(Uint128::zero(), 7_500).unwrap();
        assert_eq!(
            split,
            FeeSplitAmounts {
                treasury_amount: Uint128::zero(),
                bounty_amount: Uint128::zero(),
            }
        );
    }

    #[test]
    fn split_above_max_bps_returns_error() {
        let err = split_protocol_fee(Uint128::new(100), MAX_FEE_SHARE_BPS + 1).unwrap_err();
        assert_eq!(err, ContractError::InvalidFeeShareBps);
    }

    #[test]
    fn split_75_25_ratio() {
        let split = split_protocol_fee(Uint128::new(1000), 7_500).unwrap();
        assert_eq!(split.treasury_amount, Uint128::new(750));
        assert_eq!(split.bounty_amount, Uint128::new(250));
    }

    #[test]
    fn split_1_bps() {
        let split = split_protocol_fee(Uint128::new(10_000), 1).unwrap();
        assert_eq!(split.treasury_amount, Uint128::new(1));
        assert_eq!(split.bounty_amount, Uint128::new(9_999));
    }

    #[test]
    fn split_large_fee_no_overflow() {
        let large = Uint128::new(u128::MAX / 10_001);
        let split = split_protocol_fee(large, 5_000).unwrap();
        assert_eq!(
            split.treasury_amount,
            large * Uint128::new(5_000) / Uint128::new(10_000)
        );
        assert_eq!(split.treasury_amount + split.bounty_amount, large);
    }

    // ── get_treasury_fee_share_bps tests ────────────────────────────────

    #[test]
    fn default_share_when_nothing_set() {
        let deps = mock_dependencies();
        let share = get_treasury_fee_share_bps(deps.as_ref(), "ucredit");
        assert_eq!(share, DEFAULT_TREASURY_FEE_SHARE_BPS);
    }

    // ── accrue_protocol_fee tests ───────────────────────────────────────

    #[test]
    fn accrue_zero_fee_does_not_write() {
        let mut deps = mock_dependencies();
        let split = accrue_protocol_fee(&mut deps.as_mut(), "ucredit", Uint128::zero()).unwrap();
        assert_eq!(split.treasury_amount, Uint128::zero());
        assert_eq!(split.bounty_amount, Uint128::zero());
    }

    #[test]
    fn accrue_full_treasury_split() {
        let mut deps = mock_dependencies();
        let split = accrue_protocol_fee(&mut deps.as_mut(), "ucredit", Uint128::new(1000)).unwrap();
        assert_eq!(split.treasury_amount, Uint128::new(1000));
        assert_eq!(split.bounty_amount, Uint128::zero());

        let treasury = TREASURY_BALANCE
            .may_load(deps.as_ref().storage, "ucredit")
            .unwrap()
            .unwrap();
        assert_eq!(treasury, Uint128::new(1000));

        let bounty = BOUNTY_BALANCE
            .may_load(deps.as_ref().storage, "ucredit")
            .unwrap();
        assert!(bounty.is_none() || bounty.unwrap().is_zero());
    }

    // ── withdraw_treasury tests ─────────────────────────────────────────

    #[test]
    fn withdraw_treasury_unauthorized() {
        let mut deps = mock_dependencies();
        let owner = deps.api.addr_make("owner");
        let config = Config {
            owner: owner.clone(),
        };
        CONFIG.save(deps.as_mut().storage, &config).unwrap();
        let addr = deps.api.addr_make("not_owner");
        let err =
            withdraw_treasury(&mut deps.as_mut(), &addr, "ucredit", Uint128::new(100)).unwrap_err();
        assert_eq!(err, ContractError::Unauthorized);
    }

    #[test]
    fn withdraw_treasury_zero_amount() {
        let mut deps = mock_dependencies();
        let owner = deps.api.addr_make("owner");
        let config = Config {
            owner: owner.clone(),
        };
        CONFIG.save(deps.as_mut().storage, &config).unwrap();

        let err =
            withdraw_treasury(&mut deps.as_mut(), &owner, "ucredit", Uint128::zero()).unwrap_err();
        assert_eq!(err, ContractError::InvalidAmount);
    }

    #[test]
    fn withdraw_treasury_insufficient_balance() {
        let mut deps = mock_dependencies();
        let owner = deps.api.addr_make("owner");
        let config = Config {
            owner: owner.clone(),
        };
        CONFIG.save(deps.as_mut().storage, &config).unwrap();
        TREASURY_BALANCE
            .save(deps.as_mut().storage, "ucredit", &Uint128::new(50))
            .unwrap();

        let err = withdraw_treasury(&mut deps.as_mut(), &owner, "ucredit", Uint128::new(100))
            .unwrap_err();
        assert_eq!(err, ContractError::InsufficientTreasuryBalance);
    }

    #[test]
    fn withdraw_treasury_success() {
        let mut deps = mock_dependencies();
        let owner = deps.api.addr_make("owner");
        let config = Config {
            owner: owner.clone(),
        };
        CONFIG.save(deps.as_mut().storage, &config).unwrap();
        TREASURY_BALANCE
            .save(deps.as_mut().storage, "ucredit", &Uint128::new(200))
            .unwrap();

        let withdrawn =
            withdraw_treasury(&mut deps.as_mut(), &owner, "ucredit", Uint128::new(150)).unwrap();
        assert_eq!(withdrawn, Uint128::new(150));

        let balance = TREASURY_BALANCE
            .may_load(deps.as_ref().storage, "ucredit")
            .unwrap()
            .unwrap();
        assert_eq!(balance, Uint128::new(50));
    }

    // ── withdraw_bounty tests ───────────────────────────────────────────

    #[test]
    fn withdraw_bounty_unauthorized() {
        let mut deps = mock_dependencies();
        let owner = deps.api.addr_make("owner");
        let config = Config {
            owner: owner.clone(),
        };
        CONFIG.save(deps.as_mut().storage, &config).unwrap();
        let addr = deps.api.addr_make("not_owner");
        let err =
            withdraw_bounty(&mut deps.as_mut(), &addr, "ucredit", Uint128::new(100)).unwrap_err();
        assert_eq!(err, ContractError::Unauthorized);
    }

    #[test]
    fn withdraw_bounty_zero_amount() {
        let mut deps = mock_dependencies();
        let owner = deps.api.addr_make("owner");
        let config = Config {
            owner: owner.clone(),
        };
        CONFIG.save(deps.as_mut().storage, &config).unwrap();

        let err =
            withdraw_bounty(&mut deps.as_mut(), &owner, "ucredit", Uint128::zero()).unwrap_err();
        assert_eq!(err, ContractError::InvalidAmount);
    }

    #[test]
    fn withdraw_bounty_insufficient_balance() {
        let mut deps = mock_dependencies();
        let owner = deps.api.addr_make("owner");
        let config = Config {
            owner: owner.clone(),
        };
        CONFIG.save(deps.as_mut().storage, &config).unwrap();
        BOUNTY_BALANCE
            .save(deps.as_mut().storage, "ucredit", &Uint128::new(50))
            .unwrap();

        let err =
            withdraw_bounty(&mut deps.as_mut(), &owner, "ucredit", Uint128::new(100)).unwrap_err();
        assert_eq!(err, ContractError::InsufficientBountyBalance);
    }

    #[test]
    fn withdraw_bounty_success() {
        let mut deps = mock_dependencies();
        let owner = deps.api.addr_make("owner");
        let config = Config {
            owner: owner.clone(),
        };
        CONFIG.save(deps.as_mut().storage, &config).unwrap();
        BOUNTY_BALANCE
            .save(deps.as_mut().storage, "ucredit", &Uint128::new(200))
            .unwrap();

        let withdrawn =
            withdraw_bounty(&mut deps.as_mut(), &owner, "ucredit", Uint128::new(150)).unwrap();
        assert_eq!(withdrawn, Uint128::new(150));

        let balance = BOUNTY_BALANCE
            .may_load(deps.as_ref().storage, "ucredit")
            .unwrap()
            .unwrap();
        assert_eq!(balance, Uint128::new(50));
    }

    // ── Market-specific fee share tests ─────────────────────────────────

    #[test]
    fn per_market_share_overrides_default() {
        let mut deps = mock_dependencies();
        DEFAULT_FEE_SHARE_BPS
            .save(deps.as_mut().storage, &7_000)
            .unwrap();
        MARKET_FEE_SHARE_BPS
            .save(deps.as_mut().storage, "ustable", &3_000)
            .unwrap();

        let default_share = get_treasury_fee_share_bps(deps.as_ref(), "ucredit");
        assert_eq!(default_share, 7_000);

        let market_share = get_treasury_fee_share_bps(deps.as_ref(), "ustable");
        assert_eq!(market_share, 3_000);
    }

    #[test]
    fn accrue_with_per_market_share() {
        let mut deps = mock_dependencies();
        MARKET_FEE_SHARE_BPS
            .save(deps.as_mut().storage, "ustable", &5_000)
            .unwrap();

        let split = accrue_protocol_fee(&mut deps.as_mut(), "ustable", Uint128::new(200)).unwrap();
        assert_eq!(split.treasury_amount, Uint128::new(100));
        assert_eq!(split.bounty_amount, Uint128::new(100));

        let treasury = TREASURY_BALANCE
            .may_load(deps.as_ref().storage, "ustable")
            .unwrap()
            .unwrap();
        assert_eq!(treasury, Uint128::new(100));

        let bounty = BOUNTY_BALANCE
            .may_load(deps.as_ref().storage, "ustable")
            .unwrap()
            .unwrap();
        assert_eq!(bounty, Uint128::new(100));
    }

    #[test]
    fn multiple_markets_are_isolated() {
        let mut deps = mock_dependencies();
        MARKET_FEE_SHARE_BPS
            .save(deps.as_mut().storage, "ustable", &3_000)
            .unwrap();

        accrue_protocol_fee(&mut deps.as_mut(), "ucredit", Uint128::new(1000)).unwrap();
        accrue_protocol_fee(&mut deps.as_mut(), "ustable", Uint128::new(1000)).unwrap();

        let t_credit = TREASURY_BALANCE
            .may_load(deps.as_ref().storage, "ucredit")
            .unwrap()
            .unwrap();
        assert_eq!(t_credit, Uint128::new(1000));

        let t_stable = TREASURY_BALANCE
            .may_load(deps.as_ref().storage, "ustable")
            .unwrap()
            .unwrap();
        assert_eq!(t_stable, Uint128::new(300));

        let b_stable = BOUNTY_BALANCE
            .may_load(deps.as_ref().storage, "ustable")
            .unwrap()
            .unwrap();
        assert_eq!(b_stable, Uint128::new(700));
    }

    #[test]
    fn withdraw_per_market_isolated() {
        let mut deps = mock_dependencies();
        let owner = deps.api.addr_make("owner");
        let config = Config {
            owner: owner.clone(),
        };
        CONFIG.save(deps.as_mut().storage, &config).unwrap();

        accrue_protocol_fee(&mut deps.as_mut(), "ucredit", Uint128::new(500)).unwrap();
        accrue_protocol_fee(&mut deps.as_mut(), "ustable", Uint128::new(500)).unwrap();

        withdraw_treasury(&mut deps.as_mut(), &owner, "ucredit", Uint128::new(200)).unwrap();

        let t_credit = TREASURY_BALANCE
            .may_load(deps.as_ref().storage, "ucredit")
            .unwrap()
            .unwrap();
        assert_eq!(t_credit, Uint128::new(300));

        let t_stable = TREASURY_BALANCE
            .may_load(deps.as_ref().storage, "ustable")
            .unwrap()
            .unwrap();
        assert_eq!(t_stable, Uint128::new(500));
    }

    #[test]
    fn accrue_accumulates_over_multiple_calls() {
        let mut deps = mock_dependencies();
        accrue_protocol_fee(&mut deps.as_mut(), "ucredit", Uint128::new(100)).unwrap();
        accrue_protocol_fee(&mut deps.as_mut(), "ucredit", Uint128::new(200)).unwrap();

        let treasury = TREASURY_BALANCE
            .may_load(deps.as_ref().storage, "ucredit")
            .unwrap()
            .unwrap();
        assert_eq!(treasury, Uint128::new(300));
    }
}
