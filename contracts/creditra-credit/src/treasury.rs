//! Two-step, timelocked treasury withdrawal.
//!
//! Direct single-step withdrawals let a compromised or malicious owner key
//! drain the contract's balance in a single transaction. This module
//! requires the owner to first `propose_withdrawal`, then wait out a fixed
//! timelock before `execute_withdrawal` will actually move funds — giving
//! observers a window to notice and react to an unexpected proposal.

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, BankMsg, Coin, DepsMut, Env, MessageInfo, Response, Timestamp, Uint128};
use cw_storage_plus::Item;

use crate::error::ContractError;
use crate::state::CONFIG;

/// Delay between `propose_withdrawal` and the earliest allowed
/// `execute_withdrawal`, in seconds. 24 hours.
pub const WITHDRAWAL_TIMELOCK_SECONDS: u64 = 24 * 60 * 60;

/// A treasury withdrawal proposed by the owner, pending its timelock.
#[cw_serde]
pub struct PendingWithdrawal {
    pub to: Addr,
    pub amount: Uint128,
    pub denom: String,
    pub proposed_at: Timestamp,
    /// Earliest time `execute_withdrawal` is allowed to succeed.
    pub unlocks_at: Timestamp,
}

/// Storage for the single in-flight withdrawal proposal, if any. A new
/// `propose_withdrawal` call overwrites any existing (unexecuted) proposal
/// rather than queuing multiple — only one withdrawal can be pending at a time.
pub const PENDING_WITHDRAWAL: Item<PendingWithdrawal> = Item::new("pending_withdrawal");

/// Step 1: owner proposes a withdrawal. Starts the timelock; does not move funds.
///
/// Auth: contract owner only (`CONFIG.owner`).
pub fn execute_propose_withdrawal(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    to: String,
    amount: String,
    denom: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }

    let to_addr = deps.api.addr_validate(&to)?;
    let parsed_amount: Uint128 = amount
        .parse()
        .map_err(|_| ContractError::Std(cosmwasm_std::StdError::parse_err("Uint128", &amount)))?;
    if parsed_amount.is_zero() {
        return Err(ContractError::InvalidAmount);
    }
    if denom.is_empty() {
        return Err(ContractError::InvalidAmount);
    }

    let unlocks_at = env.block.time.plus_seconds(WITHDRAWAL_TIMELOCK_SECONDS);
    let pending = PendingWithdrawal {
        to: to_addr,
        amount: parsed_amount,
        denom,
        proposed_at: env.block.time,
        unlocks_at,
    };
    PENDING_WITHDRAWAL.save(deps.storage, &pending)?;

    Ok(Response::default()
        .add_attribute("action", "propose_withdrawal")
        .add_attribute("to", pending.to.to_string())
        .add_attribute("amount", pending.amount.to_string())
        .add_attribute("denom", pending.denom.clone())
        .add_attribute("unlocks_at", pending.unlocks_at.seconds().to_string()))
}

/// Step 2: owner executes a previously proposed withdrawal once its timelock
/// has elapsed. Sends the funds and clears the pending proposal.
///
/// Auth: contract owner only. Errors if no proposal is pending or the
/// timelock has not yet elapsed.
pub fn execute_execute_withdrawal(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }

    let pending = PENDING_WITHDRAWAL
        .may_load(deps.storage)?
        .ok_or(ContractError::NoPendingWithdrawal)?;

    if env.block.time < pending.unlocks_at {
        return Err(ContractError::WithdrawalTimelockActive);
    }

    // Clear before sending: the outbound BankMsg is only dispatched after
    // this handler returns Ok, but removing eagerly means a re-entrant or
    // retried call in the same block can't observe a stale pending proposal.
    PENDING_WITHDRAWAL.remove(deps.storage);

    let send_msg = BankMsg::Send {
        to_address: pending.to.to_string(),
        amount: vec![Coin {
            denom: pending.denom.clone(),
            amount: pending.amount,
        }],
    };

    Ok(Response::default()
        .add_message(send_msg)
        .add_attribute("action", "execute_withdrawal")
        .add_attribute("to", pending.to.to_string())
        .add_attribute("amount", pending.amount.to_string())
        .add_attribute("denom", pending.denom))
}

/// Cancels a pending withdrawal proposal without sending funds.
///
/// Auth: contract owner only. Errors if no proposal is pending.
pub fn execute_cancel_withdrawal(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized);
    }

    if PENDING_WITHDRAWAL.may_load(deps.storage)?.is_none() {
        return Err(ContractError::NoPendingWithdrawal);
    }
    PENDING_WITHDRAWAL.remove(deps.storage);

    Ok(Response::default().add_attribute("action", "cancel_withdrawal"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env, MockApi, MockQuerier, MockStorage};
    use cosmwasm_std::{Addr, OwnedDeps};

    use crate::contract;
    use crate::msg::InstantiateMsg;

    fn setup() -> (OwnedDeps<MockStorage, MockApi, MockQuerier>, Addr) {
        let mut deps = mock_dependencies();
        let owner = deps.api.addr_make("owner");
        let info = message_info(&owner, &[]);
        contract::instantiate(
            deps.as_mut(),
            mock_env(),
            info,
            InstantiateMsg {
                owner: owner.to_string(),
            },
        )
        .unwrap();
        (deps, owner)
    }

    #[test]
    fn propose_withdrawal_requires_owner() {
        let (mut deps, _owner) = setup();
        let not_owner = deps.api.addr_make("not_owner");

        let result = execute_propose_withdrawal(
            deps.as_mut(),
            mock_env(),
            message_info(&not_owner, &[]),
            "recipient".to_string(),
            "100".to_string(),
            "ucredit".to_string(),
        );
        assert_eq!(result.unwrap_err(), ContractError::Unauthorized);
    }

    #[test]
    fn propose_withdrawal_rejects_zero_amount() {
        let (mut deps, owner) = setup();
        let recipient = deps.api.addr_make("recipient");
        let result = execute_propose_withdrawal(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[]),
            recipient.to_string(),
            "0".to_string(),
            "ucredit".to_string(),
        );
        assert_eq!(result.unwrap_err(), ContractError::InvalidAmount);
    }

    #[test]
    fn execute_before_timelock_fails() {
        let (mut deps, owner) = setup();
        let env = mock_env();

        execute_propose_withdrawal(
            deps.as_mut(),
            env.clone(),
            message_info(&owner, &[]),
            deps.api.addr_make("recipient").to_string(),
            "1000".to_string(),
            "ucredit".to_string(),
        )
        .unwrap();

        // Still within the timelock window (no time has passed).
        let result = execute_execute_withdrawal(deps.as_mut(), env, message_info(&owner, &[]));
        assert_eq!(result.unwrap_err(), ContractError::WithdrawalTimelockActive);
    }

    #[test]
    fn execute_after_timelock_succeeds_and_sends_funds() {
        let (mut deps, owner) = setup();
        let propose_env = mock_env();
        let recipient = deps.api.addr_make("recipient");

        execute_propose_withdrawal(
            deps.as_mut(),
            propose_env.clone(),
            message_info(&owner, &[]),
            recipient.to_string(),
            "1000".to_string(),
            "ucredit".to_string(),
        )
        .unwrap();

        let mut later_env = propose_env;
        later_env.block.time = later_env.block.time.plus_seconds(WITHDRAWAL_TIMELOCK_SECONDS);

        let response =
            execute_execute_withdrawal(deps.as_mut(), later_env, message_info(&owner, &[])).unwrap();

        assert_eq!(response.messages.len(), 1);
        match &response.messages[0].msg {
            cosmwasm_std::CosmosMsg::Bank(BankMsg::Send { to_address, amount }) => {
                assert_eq!(to_address, recipient.as_str());
                assert_eq!(amount.len(), 1);
                assert_eq!(amount[0].denom, "ucredit");
                assert_eq!(amount[0].amount, Uint128::new(1000));
            }
            other => panic!("expected BankMsg::Send, got {other:?}"),
        }

        // The proposal is cleared after execution.
        assert!(PENDING_WITHDRAWAL.may_load(deps.as_ref().storage).unwrap().is_none());
    }

    #[test]
    fn execute_with_no_pending_withdrawal_fails() {
        let (mut deps, owner) = setup();
        let result = execute_execute_withdrawal(deps.as_mut(), mock_env(), message_info(&owner, &[]));
        assert_eq!(result.unwrap_err(), ContractError::NoPendingWithdrawal);
    }

    #[test]
    fn execute_withdrawal_requires_owner() {
        let (mut deps, owner) = setup();
        let not_owner = deps.api.addr_make("not_owner");
        let propose_env = mock_env();

        execute_propose_withdrawal(
            deps.as_mut(),
            propose_env.clone(),
            message_info(&owner, &[]),
            deps.api.addr_make("recipient").to_string(),
            "1000".to_string(),
            "ucredit".to_string(),
        )
        .unwrap();

        let mut later_env = propose_env;
        later_env.block.time = later_env.block.time.plus_seconds(WITHDRAWAL_TIMELOCK_SECONDS);

        let result =
            execute_execute_withdrawal(deps.as_mut(), later_env, message_info(&not_owner, &[]));
        assert_eq!(result.unwrap_err(), ContractError::Unauthorized);
    }

    #[test]
    fn cancel_withdrawal_clears_pending_proposal() {
        let (mut deps, owner) = setup();
        execute_propose_withdrawal(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[]),
            deps.api.addr_make("recipient").to_string(),
            "1000".to_string(),
            "ucredit".to_string(),
        )
        .unwrap();

        execute_cancel_withdrawal(deps.as_mut(), mock_env(), message_info(&owner, &[])).unwrap();

        assert!(PENDING_WITHDRAWAL.may_load(deps.as_ref().storage).unwrap().is_none());

        // Nothing left to execute or cancel again.
        let result = execute_execute_withdrawal(deps.as_mut(), mock_env(), message_info(&owner, &[]));
        assert_eq!(result.unwrap_err(), ContractError::NoPendingWithdrawal);

        let result = execute_cancel_withdrawal(deps.as_mut(), mock_env(), message_info(&owner, &[]));
        assert_eq!(result.unwrap_err(), ContractError::NoPendingWithdrawal);
    }

    #[test]
    fn a_new_proposal_replaces_an_existing_pending_one() {
        let (mut deps, owner) = setup();
        let recipient_a = deps.api.addr_make("recipient_a");
        let recipient_b = deps.api.addr_make("recipient_b");

        execute_propose_withdrawal(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[]),
            recipient_a.to_string(),
            "100".to_string(),
            "ucredit".to_string(),
        )
        .unwrap();

        execute_propose_withdrawal(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[]),
            recipient_b.to_string(),
            "200".to_string(),
            "ucredit".to_string(),
        )
        .unwrap();

        let pending = PENDING_WITHDRAWAL.load(deps.as_ref().storage).unwrap();
        assert_eq!(pending.to, recipient_b);
        assert_eq!(pending.amount, Uint128::new(200));
    }
}
