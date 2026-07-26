// SPDX-License-Identifier: MIT

//! Protocol fee skim split between treasury and bounty pools.
//!
//! When a borrower repays, the protocol fee (`ProtocolFeeBps`) is
//! skimmed from the total repayment amount into the contract and allocated
//! between two accumulators by `TreasuryFeeShareBps`:
//!
//! - **Treasury** — withdrawable to `TreasuryAddress`.
//! - **Bounty pool** — withdrawable to `BountyAddress`.
//!
//! The treasury share is computed with floor rounding; the bounty pool receives
//! the remainder so no tokens are lost to integer division.

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{
    Addr, BankMsg, Coin, Deps, DepsMut, MessageInfo, Response, StdError, StdResult, Uint128,
};
use cw_storage_plus::{Item, Map};

use crate::error::ContractError;
use crate::state::CONFIG;

/// Maximum basis points for a fee-share ratio (100 %).
pub const MAX_FEE_SHARE_BPS: u32 = 10_000;

/// Maximum basis points for a protocol fee (10 %).
pub const MAX_PROTOCOL_FEE_BPS: u32 = 1_000;

/// Default treasury share when unset: 100 % to treasury (backward compatible).
pub const DEFAULT_TREASURY_FEE_SHARE_BPS: u32 = 10_000;

/// Configured protocol fee in basis points.
pub const PROTOCOL_FEE_BPS: Item<u32> = Item::new("pf_bps");

/// Treasury fee share in basis points.
pub const TREASURY_FEE_SHARE_BPS: Item<u32> = Item::new("tfs_bps");

/// Treasury address where withdrawn fees will be sent.
pub const TREASURY_ADDRESS: Item<Addr> = Item::new("t_addr");

/// Bounty address where withdrawn bounty fees will be sent.
pub const BOUNTY_ADDRESS: Item<Addr> = Item::new("b_addr");

/// Accumulated treasury balance held in contract per denom (fees collected).
pub const TREASURY_BALANCES: Map<&str, Uint128> = Map::new("t_bals");

/// Accumulated bounty balance held in contract per denom (fees collected).
pub const BOUNTY_BALANCES: Map<&str, Uint128> = Map::new("b_bals");

/// Result of splitting a protocol fee between treasury and bounty accumulators.
#[cw_serde]
pub struct FeeSplitAmounts {
    /// Portion credited to `TreasuryBalance`.
    pub treasury_amount: Uint128,
    /// Portion credited to `BountyBalance`.
    pub bounty_amount: Uint128,
}

/// Split `total_fee` by `treasury_share_bps` in the range `0..=10_000`.
///
/// Treasury receives `floor(total_fee * treasury_share_bps / 10_000)`; the
/// bounty pool receives the remainder.
pub fn split_protocol_fee(total_fee: Uint128, treasury_share_bps: u32) -> FeeSplitAmounts {
    if total_fee.is_zero() {
        return FeeSplitAmounts {
            treasury_amount: Uint128::zero(),
            bounty_amount: Uint128::zero(),
        };
    }

    if treasury_share_bps == 0 {
        return FeeSplitAmounts {
            treasury_amount: Uint128::zero(),
            bounty_amount: total_fee,
        };
    }

    if treasury_share_bps >= MAX_FEE_SHARE_BPS {
        return FeeSplitAmounts {
            treasury_amount: total_fee,
            bounty_amount: Uint128::zero(),
        };
    }

    // Multiply and divide for precision floor.
    let treasury_amount = total_fee.multiply_ratio(treasury_share_bps, MAX_FEE_SHARE_BPS);
    let bounty_amount = total_fee.saturating_sub(treasury_amount);

    FeeSplitAmounts {
        treasury_amount,
        bounty_amount,
    }
}

/// Credit a skimmed protocol fee to treasury and bounty accumulators.
pub fn accrue_protocol_fee(
    deps: DepsMut,
    denom: &str,
    total_fee: Uint128,
) -> Result<(), ContractError> {
    if total_fee.is_zero() {
        return Ok(());
    }

    let treasury_share_bps = TREASURY_FEE_SHARE_BPS
        .may_load(deps.storage)?
        .unwrap_or(DEFAULT_TREASURY_FEE_SHARE_BPS);

    let split = split_protocol_fee(total_fee, treasury_share_bps);

    if !split.treasury_amount.is_zero() {
        let current = TREASURY_BALANCES
            .may_load(deps.storage, denom)?
            .unwrap_or_default();
        let new_bal = current
            .checked_add(split.treasury_amount)
            .map_err(|e| StdError::overflow(e))?;
        TREASURY_BALANCES.save(deps.storage, denom, &new_bal)?;
    }

    if !split.bounty_amount.is_zero() {
        let current = BOUNTY_BALANCES
            .may_load(deps.storage, denom)?
            .unwrap_or_default();
        let new_bal = current
            .checked_add(split.bounty_amount)
            .map_err(|e| StdError::overflow(e))?;
        BOUNTY_BALANCES.save(deps.storage, denom, &new_bal)?;
    }

    Ok(())
}

/// Set the protocol fee in basis points (admin only).
pub fn execute_set_protocol_fee_bps(
    deps: DepsMut,
    info: MessageInfo,
    bps: u32,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }

    if bps > MAX_PROTOCOL_FEE_BPS {
        return Err(ContractError::InvalidAmount);
    }

    PROTOCOL_FEE_BPS.save(deps.storage, &bps)?;

    Ok(Response::new()
        .add_attribute("action", "set_protocol_fee_bps")
        .add_attribute("bps", bps.to_string()))
}

/// Set the treasury share of skimmed protocol fees in basis points (admin only).
pub fn execute_set_treasury_fee_share_bps(
    deps: DepsMut,
    info: MessageInfo,
    bps: u32,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }

    if bps > MAX_FEE_SHARE_BPS {
        return Err(ContractError::InvalidAmount);
    }

    TREASURY_FEE_SHARE_BPS.save(deps.storage, &bps)?;

    Ok(Response::new()
        .add_attribute("action", "set_treasury_fee_share_bps")
        .add_attribute("bps", bps.to_string()))
}

/// Configure the treasury address (admin only).
pub fn execute_set_treasury_address(
    deps: DepsMut,
    info: MessageInfo,
    address: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }

    let addr = deps.api.addr_validate(&address)?;
    TREASURY_ADDRESS.save(deps.storage, &addr)?;

    Ok(Response::new()
        .add_attribute("action", "set_treasury_address")
        .add_attribute("address", address))
}

/// Configure the bounty pool address (admin only).
pub fn execute_set_bounty_address(
    deps: DepsMut,
    info: MessageInfo,
    address: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }

    let addr = deps.api.addr_validate(&address)?;
    BOUNTY_ADDRESS.save(deps.storage, &addr)?;

    Ok(Response::new()
        .add_attribute("action", "set_bounty_address")
        .add_attribute("address", address))
}

/// Withdraw accumulated treasury balance for a denom (admin only).
pub fn execute_withdraw_treasury(
    deps: DepsMut,
    info: MessageInfo,
    denom: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }

    let treasury_addr = TREASURY_ADDRESS
        .may_load(deps.storage)?
        .ok_or(ContractError::TreasuryAddressNotSet)?;

    let balance = TREASURY_BALANCES
        .may_load(deps.storage, &denom)?
        .unwrap_or_default();

    if balance.is_zero() {
        return Err(ContractError::InvalidAmount);
    }

    TREASURY_BALANCES.save(deps.storage, &denom, &Uint128::zero())?;

    let bank_msg = BankMsg::Send {
        to_address: treasury_addr.to_string(),
        amount: vec![Coin {
            denom: denom.clone(),
            amount: balance,
        }],
    };

    Ok(Response::new()
        .add_message(bank_msg)
        .add_attribute("action", "withdraw_treasury")
        .add_attribute("denom", denom)
        .add_attribute("amount", balance.to_string())
        .add_attribute("recipient", treasury_addr.to_string()))
}

/// Withdraw accumulated bounty balance for a denom (admin only).
pub fn execute_withdraw_bounty(
    deps: DepsMut,
    info: MessageInfo,
    denom: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }

    let bounty_addr = BOUNTY_ADDRESS
        .may_load(deps.storage)?
        .ok_or(ContractError::BountyAddressNotSet)?;

    let balance = BOUNTY_BALANCES
        .may_load(deps.storage, &denom)?
        .unwrap_or_default();

    if balance.is_zero() {
        return Err(ContractError::InvalidAmount);
    }

    BOUNTY_BALANCES.save(deps.storage, &denom, &Uint128::zero())?;

    let bank_msg = BankMsg::Send {
        to_address: bounty_addr.to_string(),
        amount: vec![Coin {
            denom: denom.clone(),
            amount: balance,
        }],
    };

    Ok(Response::new()
        .add_message(bank_msg)
        .add_attribute("action", "withdraw_bounty")
        .add_attribute("denom", denom)
        .add_attribute("amount", balance.to_string())
        .add_attribute("recipient", bounty_addr.to_string()))
}

/// Query protocol fee in basis points.
pub fn query_protocol_fee_bps(deps: Deps) -> StdResult<u32> {
    let bps = PROTOCOL_FEE_BPS.may_load(deps.storage)?.unwrap_or(0);
    Ok(bps)
}

/// Query treasury fee share in basis points.
pub fn query_treasury_fee_share_bps(deps: Deps) -> StdResult<u32> {
    let bps = TREASURY_FEE_SHARE_BPS
        .may_load(deps.storage)?
        .unwrap_or(DEFAULT_TREASURY_FEE_SHARE_BPS);
    Ok(bps)
}

/// Query treasury address.
pub fn query_treasury_address(deps: Deps) -> StdResult<Option<Addr>> {
    let addr = TREASURY_ADDRESS.may_load(deps.storage)?;
    Ok(addr)
}

/// Query bounty address.
pub fn query_bounty_address(deps: Deps) -> StdResult<Option<Addr>> {
    let addr = BOUNTY_ADDRESS.may_load(deps.storage)?;
    Ok(addr)
}

/// Query treasury balance for a denom.
pub fn query_treasury_balance(deps: Deps, denom: String) -> StdResult<Uint128> {
    let bal = TREASURY_BALANCES
        .may_load(deps.storage, &denom)?
        .unwrap_or_default();
    Ok(bal)
}

/// Query bounty balance for a denom.
pub fn query_bounty_balance(deps: Deps, denom: String) -> StdResult<Uint128> {
    let bal = BOUNTY_BALANCES
        .may_load(deps.storage, &denom)?
        .unwrap_or_default();
    Ok(bal)
}

#[cfg(test)]
mod tests {
    use cosmwasm_std::testing::{mock_dependencies, mock_info};
    use crate::state::Config;

    use super::*;

    #[test]
    fn test_split_protocol_fee() {
        // Zero fee
        let split = split_protocol_fee(Uint128::zero(), 5_000);
        assert_eq!(split.treasury_amount, Uint128::zero());
        assert_eq!(split.bounty_amount, Uint128::zero());

        // Zero treasury share
        let split = split_protocol_fee(Uint128::new(100), 0);
        assert_eq!(split.treasury_amount, Uint128::zero());
        assert_eq!(split.bounty_amount, Uint128::new(100));

        // Max treasury share
        let split = split_protocol_fee(Uint128::new(100), MAX_FEE_SHARE_BPS);
        assert_eq!(split.treasury_amount, Uint128::new(100));
        assert_eq!(split.bounty_amount, Uint128::zero());

        // Even split
        let split = split_protocol_fee(Uint128::new(100), 5_000);
        assert_eq!(split.treasury_amount, Uint128::new(50));
        assert_eq!(split.bounty_amount, Uint128::new(50));

        // Remainder rounding logic (floor rounding for treasury, remainder to bounty)
        let split = split_protocol_fee(Uint128::new(10), 3_333);
        assert_eq!(split.treasury_amount, Uint128::new(3)); // floor(10 * 3333 / 10000) = 3
        assert_eq!(split.bounty_amount, Uint128::new(7));
    }

    #[test]
    fn test_accrue_protocol_fee() {
        let mut deps = mock_dependencies();

        // Accrue zero fee does nothing
        accrue_protocol_fee(deps.as_mut(), "ucredit", Uint128::zero()).unwrap();
        assert_eq!(
            TREASURY_BALANCES
                .may_load(&deps.storage, "ucredit")
                .unwrap(),
            None
        );

        // Configure treasury fee share
        TREASURY_FEE_SHARE_BPS
            .save(&mut deps.storage, &7_000)
            .unwrap();

        accrue_protocol_fee(deps.as_mut(), "ucredit", Uint128::new(100)).unwrap();

        assert_eq!(
            TREASURY_BALANCES
                .load(&deps.storage, "ucredit")
                .unwrap(),
            Uint128::new(70)
        );
        assert_eq!(
            BOUNTY_BALANCES
                .load(&deps.storage, "ucredit")
                .unwrap(),
            Uint128::new(30)
        );

        // Accumulate balances
        accrue_protocol_fee(deps.as_mut(), "ucredit", Uint128::new(200)).unwrap();
        assert_eq!(
            TREASURY_BALANCES
                .load(&deps.storage, "ucredit")
                .unwrap(),
            Uint128::new(210) // 70 + 140
        );
        assert_eq!(
            BOUNTY_BALANCES
                .load(&deps.storage, "ucredit")
                .unwrap(),
            Uint128::new(90) // 30 + 60
        );
    }

    #[test]
    fn test_admin_config_handlers() {
        let mut deps = mock_dependencies();
        let owner = Addr::unchecked("owner");
        let non_owner = Addr::unchecked("attacker");

        CONFIG
            .save(&mut deps.storage, &Config { owner: owner.clone() })
            .unwrap();

        // 1. Set protocol fee bps
        let info = mock_info(non_owner.as_str(), &[]);
        let err = execute_set_protocol_fee_bps(deps.as_mut(), info, 100).unwrap_err();
        assert_eq!(err, ContractError::Unauthorized);

        let info = mock_info(owner.as_str(), &[]);
        let err = execute_set_protocol_fee_bps(deps.as_mut(), info.clone(), 1_001).unwrap_err();
        assert_eq!(err, ContractError::InvalidAmount);

        let res = execute_set_protocol_fee_bps(deps.as_mut(), info.clone(), 500).unwrap();
        assert_eq!(res.attributes[0].value, "set_protocol_fee_bps");
        assert_eq!(res.attributes[1].value, "500");
        assert_eq!(PROTOCOL_FEE_BPS.load(&deps.storage).unwrap(), 500);

        // 2. Set treasury fee share bps
        let info = mock_info(non_owner.as_str(), &[]);
        let err = execute_set_treasury_fee_share_bps(deps.as_mut(), info, 5_000).unwrap_err();
        assert_eq!(err, ContractError::Unauthorized);

        let info = mock_info(owner.as_str(), &[]);
        let err =
            execute_set_treasury_fee_share_bps(deps.as_mut(), info.clone(), 10_001).unwrap_err();
        assert_eq!(err, ContractError::InvalidAmount);

        let res = execute_set_treasury_fee_share_bps(deps.as_mut(), info.clone(), 6_000).unwrap();
        assert_eq!(res.attributes[0].value, "set_treasury_fee_share_bps");
        assert_eq!(res.attributes[1].value, "6000");
        assert_eq!(TREASURY_FEE_SHARE_BPS.load(&deps.storage).unwrap(), 6_000);

        // 3. Set treasury address
        let info = mock_info(non_owner.as_str(), &[]);
        let err =
            execute_set_treasury_address(deps.as_mut(), info, "treasury".to_string()).unwrap_err();
        assert_eq!(err, ContractError::Unauthorized);

        let info = mock_info(owner.as_str(), &[]);
        let res =
            execute_set_treasury_address(deps.as_mut(), info.clone(), "treasury".to_string())
                .unwrap();
        assert_eq!(res.attributes[0].value, "set_treasury_address");
        assert_eq!(res.attributes[1].value, "treasury");
        assert_eq!(
            TREASURY_ADDRESS.load(&deps.storage).unwrap(),
            Addr::unchecked("treasury")
        );

        // 4. Set bounty address
        let info = mock_info(non_owner.as_str(), &[]);
        let err = execute_set_bounty_address(deps.as_mut(), info, "bounty".to_string()).unwrap_err();
        assert_eq!(err, ContractError::Unauthorized);

        let info = mock_info(owner.as_str(), &[]);
        let res =
            execute_set_bounty_address(deps.as_mut(), info.clone(), "bounty".to_string()).unwrap();
        assert_eq!(res.attributes[0].value, "set_bounty_address");
        assert_eq!(res.attributes[1].value, "bounty");
        assert_eq!(
            BOUNTY_ADDRESS.load(&deps.storage).unwrap(),
            Addr::unchecked("bounty")
        );
    }

    #[test]
    fn test_withdraw_handlers() {
        let mut deps = mock_dependencies();
        let owner = Addr::unchecked("owner");
        let non_owner = Addr::unchecked("attacker");

        CONFIG
            .save(&mut deps.storage, &Config { owner: owner.clone() })
            .unwrap();

        // Pre-fill balances
        TREASURY_BALANCES
            .save(&mut deps.storage, "ucredit", &Uint128::new(500))
            .unwrap();
        BOUNTY_BALANCES
            .save(&mut deps.storage, "ucredit", &Uint128::new(200))
            .unwrap();

        // 1. Withdraw without addresses set
        let info = mock_info(owner.as_str(), &[]);
        let err =
            execute_withdraw_treasury(deps.as_mut(), info.clone(), "ucredit".to_string()).unwrap_err();
        assert_eq!(err, ContractError::TreasuryAddressNotSet);

        let err = execute_withdraw_bounty(deps.as_mut(), info.clone(), "ucredit".to_string())
            .unwrap_err();
        assert_eq!(err, ContractError::BountyAddressNotSet);

        // Set addresses
        TREASURY_ADDRESS
            .save(&mut deps.storage, &Addr::unchecked("treasury_recipient"))
            .unwrap();
        BOUNTY_ADDRESS
            .save(&mut deps.storage, &Addr::unchecked("bounty_recipient"))
            .unwrap();

        // 2. Withdraw by non-owner
        let info = mock_info(non_owner.as_str(), &[]);
        let err =
            execute_withdraw_treasury(deps.as_mut(), info.clone(), "ucredit".to_string()).unwrap_err();
        assert_eq!(err, ContractError::Unauthorized);

        let err = execute_withdraw_bounty(deps.as_mut(), info.clone(), "ucredit".to_string())
            .unwrap_err();
        assert_eq!(err, ContractError::Unauthorized);

        // 3. Successful withdraws
        let info = mock_info(owner.as_str(), &[]);
        let res =
            execute_withdraw_treasury(deps.as_mut(), info.clone(), "ucredit".to_string()).unwrap();
        assert_eq!(res.attributes[0].value, "withdraw_treasury");
        assert_eq!(res.attributes[1].value, "ucredit");
        assert_eq!(res.attributes[2].value, "500");
        assert_eq!(res.messages.len(), 1);
        assert_eq!(
            TREASURY_BALANCES
                .load(&deps.storage, "ucredit")
                .unwrap(),
            Uint128::zero()
        );

        let res = execute_withdraw_bounty(deps.as_mut(), info.clone(), "ucredit".to_string()).unwrap();
        assert_eq!(res.attributes[0].value, "withdraw_bounty");
        assert_eq!(res.attributes[1].value, "ucredit");
        assert_eq!(res.attributes[2].value, "200");
        assert_eq!(res.messages.len(), 1);
        assert_eq!(
            BOUNTY_BALANCES
                .load(&deps.storage, "ucredit")
                .unwrap(),
            Uint128::zero()
        );

        // 4. Withdraw zero balance
        let err =
            execute_withdraw_treasury(deps.as_mut(), info.clone(), "ucredit".to_string()).unwrap_err();
        assert_eq!(err, ContractError::InvalidAmount);

        let err = execute_withdraw_bounty(deps.as_mut(), info.clone(), "ucredit".to_string())
            .unwrap_err();
        assert_eq!(err, ContractError::InvalidAmount);
    }
}
