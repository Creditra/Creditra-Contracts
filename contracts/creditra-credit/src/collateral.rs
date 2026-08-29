use cosmwasm_std::{Addr, Deps, DepsMut, Response, Uint128};

use crate::error::ContractError;
use crate::state::{
    BORROWER_COLLATERAL_TOKENS, COLLATERAL_BALANCES, COLLATERAL_RISK_WEIGHTS,
    COLLATERAL_TOKEN_ALLOWLIST, DEFAULT_COLLATERAL_RISK_WEIGHT_BPS,
};

/// Return `true` if `denom` is in the admin-managed collateral allowlist.
///
/// When the allowlist is absent (never configured) no token is allowed.
pub fn is_collateral_token_allowed(deps: Deps, denom: &str) -> bool {
    COLLATERAL_TOKEN_ALLOWLIST
        .may_load(deps.storage)
        .unwrap_or_default()
        .map(|list| list.contains(&denom.to_string()))
        .unwrap_or(false)
}

/// Return the effective risk weight for `denom`.
///
/// Uses the per-token override when present, otherwise falls back to
/// [`DEFAULT_COLLATERAL_RISK_WEIGHT_BPS`] (100 %).
pub fn collateral_risk_weight_bps(deps: Deps, denom: &str) -> u32 {
    COLLATERAL_RISK_WEIGHTS
        .may_load(deps.storage, denom)
        .unwrap_or(None)
        .unwrap_or(DEFAULT_COLLATERAL_RISK_WEIGHT_BPS)
}

/// Deposit `amount` of `denom` as collateral on behalf of `borrower`.
///
/// # Authorization
///
/// Only the admin (owner) may call this entry point — it is a privileged
/// operation that records the collateral on-chain.  The actual token transfer
/// is expected to happen off-chain or through a separate settlement contract.
///
/// # Errors
///
/// - [`ContractError::CollateralTokenNotAllowed`] if `denom` is not in the
///   allowlist.
/// - [`ContractError::InvalidAmount`] if `amount` is zero.
/// - [`ContractError::Overflow`] if the new balance would overflow `Uint128`.
pub fn deposit_collateral(
    deps: DepsMut,
    borrower: &Addr,
    denom: &str,
    amount: Uint128,
) -> Result<Response, ContractError> {
    if amount.is_zero() {
        return Err(ContractError::InvalidAmount);
    }
    if !is_collateral_token_allowed(deps.as_ref(), denom) {
        return Err(ContractError::CollateralTokenNotAllowed);
    }

    let key = (borrower, denom);
    let balance = COLLATERAL_BALANCES
        .may_load(deps.storage, key)?
        .unwrap_or_default();
    let new_balance = balance.checked_add(amount).map_err(|_| ContractError::Overflow)?;
    COLLATERAL_BALANCES.save(deps.storage, key, &new_balance)?;

    let mut tokens = BORROWER_COLLATERAL_TOKENS
        .may_load(deps.storage, borrower)?
        .unwrap_or_default();
    if !tokens.contains(&denom.to_string()) {
        tokens.push(denom.to_string());
        BORROWER_COLLATERAL_TOKENS.save(deps.storage, borrower, &tokens)?;
    }

    Ok(Response::default()
        .add_attribute("action", "deposit_collateral")
        .add_attribute("borrower", borrower.as_str())
        .add_attribute("denom", denom)
        .add_attribute("amount", amount.to_string()))
}

/// Withdraw `amount` of `denom` collateral for `borrower`.
///
/// # Authorization
///
/// Only the admin (owner) may call this entry point.
///
/// # Errors
///
/// - [`ContractError::InsufficientCollateralBalance`] if the borrower has
///   less than `amount` deposited for `denom`.
/// - [`ContractError::InvalidAmount`] if `amount` is zero.
pub fn withdraw_collateral(
    deps: DepsMut,
    borrower: &Addr,
    denom: &str,
    amount: Uint128,
) -> Result<Response, ContractError> {
    if amount.is_zero() {
        return Err(ContractError::InvalidAmount);
    }

    let key = (borrower, denom);
    let balance = COLLATERAL_BALANCES
        .may_load(deps.storage, key)?
        .ok_or(ContractError::InsufficientCollateralBalance)?;

    if amount > balance {
        return Err(ContractError::InsufficientCollateralBalance);
    }

    let new_balance = balance
        .checked_sub(amount)
        .map_err(|_| ContractError::Overflow)?;

    if new_balance.is_zero() {
        COLLATERAL_BALANCES.remove(deps.storage, key);
        let mut tokens = BORROWER_COLLATERAL_TOKENS
            .may_load(deps.storage, borrower)?
            .unwrap_or_default();
        tokens.retain(|t| t != denom);
        BORROWER_COLLATERAL_TOKENS.save(deps.storage, borrower, &tokens)?;
    } else {
        COLLATERAL_BALANCES.save(deps.storage, key, &new_balance)?;
    }

    Ok(Response::default()
        .add_attribute("action", "withdraw_collateral")
        .add_attribute("borrower", borrower.as_str())
        .add_attribute("denom", denom)
        .add_attribute("amount", amount.to_string()))
}

/// Return the raw deposited balance of `denom` for `borrower`.
///
/// Returns `Uint128::zero()` when no balance exists (equivalent to a missing
/// storage entry).
pub fn query_collateral_balance(deps: Deps, borrower: &Addr, denom: &str) -> Uint128 {
    COLLATERAL_BALANCES
        .may_load(deps.storage, (borrower, denom))
        .unwrap_or_default()
        .unwrap_or_default()
}

/// Return all token denominations and balances deposited by `borrower`.
///
/// The returned vector is sorted by denomination for deterministic output.
pub fn query_borrower_collateral(
    deps: Deps,
    borrower: &Addr,
) -> Vec<(String, Uint128)> {
    let tokens = BORROWER_COLLATERAL_TOKENS
        .may_load(deps.storage, borrower)
        .unwrap_or_default()
        .unwrap_or_default();

    let mut result: Vec<(String, Uint128)> = tokens
        .iter()
        .map(|t| {
            let balance = query_collateral_balance(deps, borrower, t);
            (t.clone(), balance)
        })
        .filter(|(_, balance)| !balance.is_zero())
        .collect();
    result.sort_by(|a, b| a.0.cmp(&b.0));
    result
}

/// Compute the risk-weighted total collateral value for `borrower`.
///
/// Each token's balance is multiplied by its risk weight (bps) and divided by
/// 10_000.  The result is the sum across all tokens, usable in health-factor
/// and proof-of-reserve computations.
///
/// # Errors
///
/// Returns [`ContractError::Overflow`] if any intermediate multiplication
/// overflows `Uint128`.
pub fn weighted_collateral_total(deps: Deps, borrower: &Addr) -> Result<Uint128, ContractError> {
    let collateral = query_borrower_collateral(deps, borrower);
    let mut total = Uint128::zero();
    for (denom, balance) in &collateral {
        let weight = collateral_risk_weight_bps(deps, denom);
        let weighted = balance
            .checked_mul(Uint128::from(weight))
            .map_err(|_| ContractError::Overflow)?
            .checked_div(Uint128::from(10_000u32))
            .map_err(|_| ContractError::Overflow)?;
        total = total.checked_add(weighted).map_err(|_| ContractError::Overflow)?;
    }
    Ok(total)
}

/// Return the full allowlist of accepted collateral denominations.
pub fn query_collateral_allowlist(deps: Deps) -> Vec<String> {
    COLLATERAL_TOKEN_ALLOWLIST
        .may_load(deps.storage)
        .unwrap_or_default()
        .unwrap_or_default()
}

/// Add a denomination to the collateral allowlist with an optional risk weight.
///
/// # Errors
///
/// - [`ContractError::InvalidAmount`] if `risk_weight_bps > 10_000`.
/// - [`ContractError::AlreadySettled`] if `denom` is already in the allowlist.
pub fn add_collateral_token(
    deps: DepsMut,
    denom: &str,
    risk_weight_bps: u32,
) -> Result<Response, ContractError> {
    if risk_weight_bps > 10_000 {
        return Err(ContractError::InvalidAmount);
    }

    let mut list = COLLATERAL_TOKEN_ALLOWLIST
        .may_load(deps.storage)?
        .unwrap_or_default();

    if list.contains(&denom.to_string()) {
        return Err(ContractError::AlreadySettled);
    }

    list.push(denom.to_string());
    COLLATERAL_TOKEN_ALLOWLIST.save(deps.storage, &list)?;

    if risk_weight_bps != DEFAULT_COLLATERAL_RISK_WEIGHT_BPS {
        COLLATERAL_RISK_WEIGHTS.save(deps.storage, denom, &risk_weight_bps)?;
    }

    Ok(Response::default()
        .add_attribute("action", "add_collateral_token")
        .add_attribute("denom", denom)
        .add_attribute("risk_weight_bps", risk_weight_bps.to_string()))
}

/// Remove a denomination from the collateral allowlist.
///
/// Existing deposits of this token remain in storage but new deposits are
/// rejected.  The risk weight entry is also removed.
pub fn remove_collateral_token(
    deps: DepsMut,
    denom: &str,
) -> Result<Response, ContractError> {
    let mut list = COLLATERAL_TOKEN_ALLOWLIST
        .may_load(deps.storage)?
        .unwrap_or_default();

    if !list.contains(&denom.to_string()) {
        return Err(ContractError::CollateralTokenNotAllowed);
    }

    list.retain(|d| d != denom);
    COLLATERAL_TOKEN_ALLOWLIST.save(deps.storage, &list)?;
    COLLATERAL_RISK_WEIGHTS.remove(deps.storage, denom);

    Ok(Response::default()
        .add_attribute("action", "remove_collateral_token")
        .add_attribute("denom", denom))
}

/// Update an existing token's risk weight.
///
/// # Errors
///
/// - [`ContractError::CollateralTokenNotAllowed`] if `denom` is not in the
///   allowlist.
/// - [`ContractError::InvalidAmount`] if `risk_weight_bps > 10_000`.
pub fn set_collateral_risk_weight(
    deps: DepsMut,
    denom: &str,
    risk_weight_bps: u32,
) -> Result<Response, ContractError> {
    if risk_weight_bps > 10_000 {
        return Err(ContractError::InvalidAmount);
    }

    let list = COLLATERAL_TOKEN_ALLOWLIST
        .may_load(deps.storage)?
        .unwrap_or_default();

    if !list.contains(&denom.to_string()) {
        return Err(ContractError::CollateralTokenNotAllowed);
    }

    COLLATERAL_RISK_WEIGHTS.save(deps.storage, denom, &risk_weight_bps)?;

    Ok(Response::default()
        .add_attribute("action", "set_collateral_risk_weight")
        .add_attribute("denom", denom)
        .add_attribute("risk_weight_bps", risk_weight_bps.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{mock_dependencies, MockApi, MockQuerier, MockStorage};
    use cosmwasm_std::{Addr, OwnedDeps};

    fn alice() -> Addr {
        Addr::unchecked("alice")
    }

    fn bob() -> Addr {
        Addr::unchecked("bob")
    }

    fn setup_allowlist(deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>, denoms: &[&str]) {
        let list: Vec<String> = denoms.iter().map(|d| d.to_string()).collect();
        COLLATERAL_TOKEN_ALLOWLIST
            .save(deps.as_mut().storage, &list)
            .unwrap();
    }

    fn setup_deposit(
        deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
        borrower: &Addr,
        denom: &str,
        amount: u128,
    ) {
        setup_allowlist(deps, &[denom]);
        deposit_collateral(
            deps.as_mut(),
            borrower,
            denom,
            Uint128::new(amount),
        )
        .unwrap();
    }

    mod is_collateral_token_allowed {
        use super::*;

        #[test]
        fn returns_false_when_allowlist_is_empty() {
            let deps = mock_dependencies();
            assert!(!is_collateral_token_allowed(deps.as_ref(), "uusd"));
        }

        #[test]
        fn returns_false_when_allowlist_is_absent() {
            let deps = mock_dependencies();
            assert!(!is_collateral_token_allowed(deps.as_ref(), "uusd"));
        }

        #[test]
        fn returns_true_for_allowed_token() {
            let mut deps = mock_dependencies();
            setup_allowlist(&mut deps, &["uusd", "uatom"]);
            assert!(is_collateral_token_allowed(deps.as_ref(), "uusd"));
            assert!(is_collateral_token_allowed(deps.as_ref(), "uatom"));
        }

        #[test]
        fn returns_false_for_unlisted_token() {
            let mut deps = mock_dependencies();
            setup_allowlist(&mut deps, &["uusd"]);
            assert!(!is_collateral_token_allowed(deps.as_ref(), "uatom"));
        }

        #[test]
        fn case_sensitive_matching() {
            let mut deps = mock_dependencies();
            setup_allowlist(&mut deps, &["uUSD"]);
            assert!(is_collateral_token_allowed(deps.as_ref(), "uUSD"));
            assert!(!is_collateral_token_allowed(deps.as_ref(), "uusd"));
        }
    }

    mod collateral_risk_weight_bps {
        use super::*;

        #[test]
        fn returns_default_when_not_configured() {
            let deps = mock_dependencies();
            assert_eq!(
                collateral_risk_weight_bps(deps.as_ref(), "uusd"),
                DEFAULT_COLLATERAL_RISK_WEIGHT_BPS
            );
        }

        #[test]
        fn returns_configured_value() {
            let mut deps = mock_dependencies();
            COLLATERAL_RISK_WEIGHTS
                .save(deps.as_mut().storage, "uatom", &7_500)
                .unwrap();
            assert_eq!(collateral_risk_weight_bps(deps.as_ref(), "uatom"), 7_500);
            assert_eq!(
                collateral_risk_weight_bps(deps.as_ref(), "uusd"),
                DEFAULT_COLLATERAL_RISK_WEIGHT_BPS
            );
        }
    }

    mod deposit_collateral {
        use super::*;

        #[test]
        fn rejects_zero_amount() {
            let mut deps = mock_dependencies();
            setup_allowlist(&mut deps, &["uusd"]);
            let err = deposit_collateral(deps.as_mut(), &alice(), "uusd", Uint128::zero())
                .unwrap_err();
            assert_eq!(err, ContractError::InvalidAmount);
        }

        #[test]
        fn rejects_unallowed_token() {
            let mut deps = mock_dependencies();
            let err = deposit_collateral(
                deps.as_mut(),
                &alice(),
                "uusd",
                Uint128::new(100),
            )
            .unwrap_err();
            assert_eq!(err, ContractError::CollateralTokenNotAllowed);
        }

        #[test]
        fn records_deposit() {
            let mut deps = mock_dependencies();
            setup_allowlist(&mut deps, &["uusd"]);

            deposit_collateral(deps.as_mut(), &alice(), "uusd", Uint128::new(1000)).unwrap();

            let balance = COLLATERAL_BALANCES
                .may_load(deps.as_ref().storage, (&alice(), "uusd"))
                .unwrap()
                .unwrap();
            assert_eq!(balance, Uint128::new(1000));
        }

        #[test]
        fn accumulates_multiple_deposits() {
            let mut deps = mock_dependencies();
            setup_allowlist(&mut deps, &["uusd"]);

            deposit_collateral(deps.as_mut(), &alice(), "uusd", Uint128::new(500)).unwrap();
            deposit_collateral(deps.as_mut(), &alice(), "uusd", Uint128::new(300)).unwrap();

            let balance = COLLATERAL_BALANCES
                .may_load(deps.as_ref().storage, (&alice(), "uusd"))
                .unwrap()
                .unwrap();
            assert_eq!(balance, Uint128::new(800));
        }

        #[test]
        fn tracks_borrower_tokens() {
            let mut deps = mock_dependencies();
            setup_allowlist(&mut deps, &["uusd", "uatom"]);

            deposit_collateral(deps.as_mut(), &alice(), "uusd", Uint128::new(500)).unwrap();
            deposit_collateral(deps.as_mut(), &alice(), "uatom", Uint128::new(300)).unwrap();

            let tokens = BORROWER_COLLATERAL_TOKENS
                .may_load(deps.as_ref().storage, &alice())
                .unwrap()
                .unwrap();
            assert_eq!(tokens.len(), 2);
            assert!(tokens.contains(&"uusd".to_string()));
            assert!(tokens.contains(&"uatom".to_string()));
        }

        #[test]
        fn isolates_borrowers() {
            let mut deps = mock_dependencies();
            setup_allowlist(&mut deps, &["uusd"]);

            deposit_collateral(deps.as_mut(), &alice(), "uusd", Uint128::new(500)).unwrap();

            let alice_balance = COLLATERAL_BALANCES
                .may_load(deps.as_ref().storage, (&alice(), "uusd"))
                .unwrap()
                .unwrap();
            assert_eq!(alice_balance, Uint128::new(500));

            let bob_balance = COLLATERAL_BALANCES
                .may_load(deps.as_ref().storage, (&bob(), "uusd"))
                .unwrap();
            assert!(bob_balance.is_none());
        }

        #[test]
        fn response_has_correct_attributes() {
            let mut deps = mock_dependencies();
            setup_allowlist(&mut deps, &["uusd"]);

            let resp =
                deposit_collateral(deps.as_mut(), &alice(), "uusd", Uint128::new(100)).unwrap();
            let attrs = resp.attributes;
            assert_eq!(attrs[0].value, "deposit_collateral");
            assert_eq!(attrs[1].value, "alice");
            assert_eq!(attrs[2].value, "uusd");
            assert_eq!(attrs[3].value, "100");
        }

        #[test]
        fn does_not_duplicate_token_in_borrower_list() {
            let mut deps = mock_dependencies();
            setup_allowlist(&mut deps, &["uusd"]);

            deposit_collateral(deps.as_mut(), &alice(), "uusd", Uint128::new(100)).unwrap();
            deposit_collateral(deps.as_mut(), &alice(), "uusd", Uint128::new(50)).unwrap();

            let tokens = BORROWER_COLLATERAL_TOKENS
                .may_load(deps.as_ref().storage, &alice())
                .unwrap()
                .unwrap();
            assert_eq!(tokens.len(), 1);
        }
    }

    mod withdraw_collateral {
        use super::*;

        #[test]
        fn rejects_zero_amount() {
            let mut deps = mock_dependencies();
            let err = withdraw_collateral(deps.as_mut(), &alice(), "uusd", Uint128::zero())
                .unwrap_err();
            assert_eq!(err, ContractError::InvalidAmount);
        }

        #[test]
        fn rejects_insufficient_balance() {
            let mut deps = mock_dependencies();
            let err = withdraw_collateral(deps.as_mut(), &alice(), "uusd", Uint128::new(100))
                .unwrap_err();
            assert_eq!(err, ContractError::InsufficientCollateralBalance);
        }

        #[test]
        fn withdraws_partial_amount() {
            let mut deps = mock_dependencies();
            setup_deposit(&mut deps, &alice(), "uusd", 1000);

            withdraw_collateral(deps.as_mut(), &alice(), "uusd", Uint128::new(300)).unwrap();

            let balance = COLLATERAL_BALANCES
                .may_load(deps.as_ref().storage, (&alice(), "uusd"))
                .unwrap()
                .unwrap();
            assert_eq!(balance, Uint128::new(700));
        }

        #[test]
        fn withdraws_full_amount_and_cleans_up() {
            let mut deps = mock_dependencies();
            setup_deposit(&mut deps, &alice(), "uusd", 500);

            withdraw_collateral(deps.as_mut(), &alice(), "uusd", Uint128::new(500)).unwrap();

            let balance = COLLATERAL_BALANCES
                .may_load(deps.as_ref().storage, (&alice(), "uusd"))
                .unwrap();
            assert!(balance.is_none());

            let tokens = BORROWER_COLLATERAL_TOKENS
                .may_load(deps.as_ref().storage, &alice())
                .unwrap()
                .unwrap();
            assert!(!tokens.contains(&"uusd".to_string()));
        }

        #[test]
        fn rejects_withdraw_exceeding_balance() {
            let mut deps = mock_dependencies();
            setup_deposit(&mut deps, &alice(), "uusd", 100);

            let err = withdraw_collateral(deps.as_mut(), &alice(), "uusd", Uint128::new(101))
                .unwrap_err();
            assert_eq!(err, ContractError::InsufficientCollateralBalance);
        }

        #[test]
        fn response_has_correct_attributes() {
            let mut deps = mock_dependencies();
            setup_deposit(&mut deps, &alice(), "uusd", 500);

            let resp =
                withdraw_collateral(deps.as_mut(), &alice(), "uusd", Uint128::new(200)).unwrap();
            let attrs = resp.attributes;
            assert_eq!(attrs[0].value, "withdraw_collateral");
            assert_eq!(attrs[1].value, "alice");
            assert_eq!(attrs[2].value, "uusd");
            assert_eq!(attrs[3].value, "200");
        }

        #[test]
        fn preserves_other_tokens_when_one_is_depleted() {
            let mut deps = mock_dependencies();
            setup_allowlist(&mut deps, &["uusd", "uatom"]);

            deposit_collateral(deps.as_mut(), &alice(), "uusd", Uint128::new(100)).unwrap();
            deposit_collateral(deps.as_mut(), &alice(), "uatom", Uint128::new(200)).unwrap();

            withdraw_collateral(deps.as_mut(), &alice(), "uusd", Uint128::new(100)).unwrap();

            let balance = COLLATERAL_BALANCES
                .may_load(deps.as_ref().storage, (&alice(), "uatom"))
                .unwrap()
                .unwrap();
            assert_eq!(balance, Uint128::new(200));

            let tokens = BORROWER_COLLATERAL_TOKENS
                .may_load(deps.as_ref().storage, &alice())
                .unwrap()
                .unwrap();
            assert!(!tokens.contains(&"uusd".to_string()));
            assert!(tokens.contains(&"uatom".to_string()));
        }
    }

    mod query_collateral_balance {
        use super::*;

        #[test]
        fn returns_zero_for_missing_balance() {
            let deps = mock_dependencies();
            let balance = query_collateral_balance(deps.as_ref(), &alice(), "uusd");
            assert_eq!(balance, Uint128::zero());
        }

        #[test]
        fn returns_stored_balance() {
            let mut deps = mock_dependencies();
            setup_deposit(&mut deps, &alice(), "uusd", 750);
            let balance = query_collateral_balance(deps.as_ref(), &alice(), "uusd");
            assert_eq!(balance, Uint128::new(750));
        }

        #[test]
        fn returns_zero_for_different_borrower() {
            let mut deps = mock_dependencies();
            setup_deposit(&mut deps, &alice(), "uusd", 750);
            let balance = query_collateral_balance(deps.as_ref(), &bob(), "uusd");
            assert_eq!(balance, Uint128::zero());
        }
    }

    mod query_borrower_collateral {
        use super::*;

        #[test]
        fn returns_empty_for_borrower_with_no_deposits() {
            let deps = mock_dependencies();
            let result = query_borrower_collateral(deps.as_ref(), &alice());
            assert!(result.is_empty());
        }

        #[test]
        fn returns_all_tokens_sorted() {
            let mut deps = mock_dependencies();
            setup_allowlist(&mut deps, &["uatom", "uusd", "uosmo"]);

            deposit_collateral(deps.as_mut(), &alice(), "uosmo", Uint128::new(300)).unwrap();
            deposit_collateral(deps.as_mut(), &alice(), "uusd", Uint128::new(500)).unwrap();
            deposit_collateral(deps.as_mut(), &alice(), "uatom", Uint128::new(100)).unwrap();

            let result = query_borrower_collateral(deps.as_ref(), &alice());
            assert_eq!(result.len(), 3);
            assert_eq!(result[0].0, "uatom");
            assert_eq!(result[0].1, Uint128::new(100));
            assert_eq!(result[1].0, "uosmo");
            assert_eq!(result[1].1, Uint128::new(300));
            assert_eq!(result[2].0, "uusd");
            assert_eq!(result[2].1, Uint128::new(500));
        }

        #[test]
        fn isolates_borrowers() {
            let mut deps = mock_dependencies();
            setup_allowlist(&mut deps, &["uusd"]);

            deposit_collateral(deps.as_mut(), &alice(), "uusd", Uint128::new(500)).unwrap();

            assert_eq!(query_borrower_collateral(deps.as_ref(), &alice()).len(), 1);
            assert!(query_borrower_collateral(deps.as_ref(), &bob()).is_empty());
        }
    }

    mod weighted_collateral_total {
        use super::*;

        #[test]
        fn returns_zero_when_no_deposits() {
            let deps = mock_dependencies();
            let total = weighted_collateral_total(deps.as_ref(), &alice()).unwrap();
            assert_eq!(total, Uint128::zero());
        }

        #[test]
        fn returns_full_value_at_default_weight() {
            let mut deps = mock_dependencies();
            setup_allowlist(&mut deps, &["uusd"]);
            deposit_collateral(deps.as_mut(), &alice(), "uusd", Uint128::new(1000)).unwrap();

            let total = weighted_collateral_total(deps.as_ref(), &alice()).unwrap();
            assert_eq!(total, Uint128::new(1000));
        }

        #[test]
        fn applies_risk_weight_correctly() {
            let mut deps = mock_dependencies();
            setup_allowlist(&mut deps, &["uatom", "uusd"]);

            COLLATERAL_RISK_WEIGHTS
                .save(deps.as_mut().storage, "uatom", &5_000)
                .unwrap();

            deposit_collateral(deps.as_mut(), &alice(), "uatom", Uint128::new(200)).unwrap();
            deposit_collateral(deps.as_mut(), &alice(), "uusd", Uint128::new(100)).unwrap();

            // uatom: 200 * 5000 / 10000 = 100
            // uusd:  100 * 10000 / 10000 = 100
            // total: 200
            let total = weighted_collateral_total(deps.as_ref(), &alice()).unwrap();
            assert_eq!(total, Uint128::new(200));
        }

        #[test]
        fn zero_weight_token_contributes_nothing() {
            let mut deps = mock_dependencies();
            setup_allowlist(&mut deps, &["uatom"]);

            COLLATERAL_RISK_WEIGHTS
                .save(deps.as_mut().storage, "uatom", &0)
                .unwrap();

            deposit_collateral(deps.as_mut(), &alice(), "uatom", Uint128::new(500)).unwrap();

            let total = weighted_collateral_total(deps.as_ref(), &alice()).unwrap();
            assert_eq!(total, Uint128::zero());
        }
    }

    mod add_collateral_token {
        use super::*;

        #[test]
        fn adds_token_to_allowlist() {
            let mut deps = mock_dependencies();
            add_collateral_token(deps.as_mut(), "uusd", 10_000).unwrap();

            let list = COLLATERAL_TOKEN_ALLOWLIST
                .may_load(deps.as_ref().storage)
                .unwrap()
                .unwrap();
            assert!(list.contains(&"uusd".to_string()));
        }

        #[test]
        fn rejects_duplicate_token() {
            let mut deps = mock_dependencies();
            add_collateral_token(deps.as_mut(), "uusd", 10_000).unwrap();
            let err = add_collateral_token(deps.as_mut(), "uusd", 10_000).unwrap_err();
            assert_eq!(err, ContractError::AlreadySettled);
        }

        #[test]
        fn rejects_risk_weight_over_max() {
            let mut deps = mock_dependencies();
            let err =
                add_collateral_token(deps.as_mut(), "uusd", 10_001).unwrap_err();
            assert_eq!(err, ContractError::InvalidAmount);
        }

        #[test]
        fn stores_non_default_risk_weight() {
            let mut deps = mock_dependencies();
            add_collateral_token(deps.as_mut(), "uatom", 7_500).unwrap();

            let weight = COLLATERAL_RISK_WEIGHTS
                .may_load(deps.as_ref().storage, "uatom")
                .unwrap()
                .unwrap();
            assert_eq!(weight, 7_500);
        }

        #[test]
        fn does_not_store_default_risk_weight() {
            let mut deps = mock_dependencies();
            add_collateral_token(deps.as_mut(), "uusd", 10_000).unwrap();

            let weight = COLLATERAL_RISK_WEIGHTS
                .may_load(deps.as_ref().storage, "uusd")
                .unwrap();
            assert!(weight.is_none());
        }

        #[test]
        fn response_has_correct_attributes() {
            let mut deps = mock_dependencies();
            let resp = add_collateral_token(deps.as_mut(), "uusd", 7_500).unwrap();
            let attrs = resp.attributes;
            assert_eq!(attrs[0].value, "add_collateral_token");
            assert_eq!(attrs[1].value, "uusd");
            assert_eq!(attrs[2].value, "7500");
        }
    }

    mod remove_collateral_token {
        use super::*;

        #[test]
        fn removes_token_from_allowlist() {
            let mut deps = mock_dependencies();
            add_collateral_token(deps.as_mut(), "uusd", 10_000).unwrap();
            remove_collateral_token(deps.as_mut(), "uusd").unwrap();

            let list = COLLATERAL_TOKEN_ALLOWLIST
                .may_load(deps.as_ref().storage)
                .unwrap()
                .unwrap();
            assert!(!list.contains(&"uusd".to_string()));
        }

        #[test]
        fn removes_risk_weight() {
            let mut deps = mock_dependencies();
            add_collateral_token(deps.as_mut(), "uatom", 7_500).unwrap();
            remove_collateral_token(deps.as_mut(), "uatom").unwrap();

            let weight = COLLATERAL_RISK_WEIGHTS
                .may_load(deps.as_ref().storage, "uatom")
                .unwrap();
            assert!(weight.is_none());
        }

        #[test]
        fn rejects_removing_unlisted_token() {
            let mut deps = mock_dependencies();
            let err = remove_collateral_token(deps.as_mut(), "uusd").unwrap_err();
            assert_eq!(err, ContractError::CollateralTokenNotAllowed);
        }

        #[test]
        fn response_has_correct_attributes() {
            let mut deps = mock_dependencies();
            add_collateral_token(deps.as_mut(), "uusd", 10_000).unwrap();
            let resp = remove_collateral_token(deps.as_mut(), "uusd").unwrap();
            let attrs = resp.attributes;
            assert_eq!(attrs[0].value, "remove_collateral_token");
            assert_eq!(attrs[1].value, "uusd");
        }
    }

    mod set_collateral_risk_weight {
        use super::*;

        #[test]
        fn updates_risk_weight() {
            let mut deps = mock_dependencies();
            add_collateral_token(deps.as_mut(), "uatom", 10_000).unwrap();
            set_collateral_risk_weight(deps.as_mut(), "uatom", 5_000).unwrap();

            let weight = COLLATERAL_RISK_WEIGHTS
                .may_load(deps.as_ref().storage, "uatom")
                .unwrap()
                .unwrap();
            assert_eq!(weight, 5_000);
        }

        #[test]
        fn rejects_unlisted_token() {
            let mut deps = mock_dependencies();
            let err = set_collateral_risk_weight(deps.as_mut(), "uusd", 5_000).unwrap_err();
            assert_eq!(err, ContractError::CollateralTokenNotAllowed);
        }

        #[test]
        fn rejects_weight_over_max() {
            let mut deps = mock_dependencies();
            add_collateral_token(deps.as_mut(), "uusd", 10_000).unwrap();
            let err = set_collateral_risk_weight(deps.as_mut(), "uusd", 10_001).unwrap_err();
            assert_eq!(err, ContractError::InvalidAmount);
        }

        #[test]
        fn response_has_correct_attributes() {
            let mut deps = mock_dependencies();
            add_collateral_token(deps.as_mut(), "uatom", 10_000).unwrap();
            let resp = set_collateral_risk_weight(deps.as_mut(), "uatom", 7_500).unwrap();
            let attrs = resp.attributes;
            assert_eq!(attrs[0].value, "set_collateral_risk_weight");
            assert_eq!(attrs[1].value, "uatom");
            assert_eq!(attrs[2].value, "7500");
        }
    }

    mod query_collateral_allowlist {
        use super::*;

        #[test]
        fn returns_empty_when_not_configured() {
            let deps = mock_dependencies();
            let list = query_collateral_allowlist(deps.as_ref());
            assert!(list.is_empty());
        }

        #[test]
        fn returns_allowed_denoms() {
            let mut deps = mock_dependencies();
            add_collateral_token(deps.as_mut(), "uusd", 10_000).unwrap();
            add_collateral_token(deps.as_mut(), "uatom", 7_500).unwrap();

            let list = query_collateral_allowlist(deps.as_ref());
            assert_eq!(list.len(), 2);
            assert!(list.contains(&"uusd".to_string()));
            assert!(list.contains(&"uatom".to_string()));
        }
    }

    mod integration {
        use super::*;

        #[test]
        fn deposit_withdraw_multiple_tokens_flow() {
            let mut deps = mock_dependencies();
            setup_allowlist(&mut deps, &["uusd", "uatom", "uosmo"]);

            deposit_collateral(deps.as_mut(), &alice(), "uusd", Uint128::new(1000)).unwrap();
            deposit_collateral(deps.as_mut(), &alice(), "uatom", Uint128::new(500)).unwrap();

            assert_eq!(
                query_collateral_balance(deps.as_ref(), &alice(), "uusd"),
                Uint128::new(1000)
            );
            assert_eq!(
                query_collateral_balance(deps.as_ref(), &alice(), "uatom"),
                Uint128::new(500)
            );

            withdraw_collateral(deps.as_mut(), &alice(), "uusd", Uint128::new(600)).unwrap();
            assert_eq!(
                query_collateral_balance(deps.as_ref(), &alice(), "uusd"),
                Uint128::new(400)
            );

            let portfolio = query_borrower_collateral(deps.as_ref(), &alice());
            assert_eq!(portfolio.len(), 2);
        }

        #[test]
        fn allowlist_governs_what_can_be_deposited() {
            let mut deps = mock_dependencies();
            setup_allowlist(&mut deps, &["uusd"]);

            deposit_collateral(deps.as_mut(), &alice(), "uusd", Uint128::new(100)).unwrap();
            let err = deposit_collateral(deps.as_mut(), &bob(), "uatom", Uint128::new(100))
                .unwrap_err();
            assert_eq!(err, ContractError::CollateralTokenNotAllowed);
        }

        #[test]
        fn removing_token_prevents_new_deposits_but_preserves_existing() {
            let mut deps = mock_dependencies();
            setup_allowlist(&mut deps, &["uusd"]);

            deposit_collateral(deps.as_mut(), &alice(), "uusd", Uint128::new(500)).unwrap();

            remove_collateral_token(deps.as_mut(), "uusd").unwrap();

            let err = deposit_collateral(deps.as_mut(), &bob(), "uusd", Uint128::new(100))
                .unwrap_err();
            assert_eq!(err, ContractError::CollateralTokenNotAllowed);

            assert_eq!(
                query_collateral_balance(deps.as_ref(), &alice(), "uusd"),
                Uint128::new(500)
            );
        }

        #[test]
        fn risk_weight_affects_weighted_total() {
            let mut deps = mock_dependencies();
            setup_allowlist(&mut deps, &["uatom"]);

            deposit_collateral(deps.as_mut(), &alice(), "uatom", Uint128::new(1000)).unwrap();

            assert_eq!(
                weighted_collateral_total(deps.as_ref(), &alice()).unwrap(),
                Uint128::new(1000)
            );

            set_collateral_risk_weight(deps.as_mut(), "uatom", 5_000).unwrap();

            assert_eq!(
                weighted_collateral_total(deps.as_ref(), &alice()).unwrap(),
                Uint128::new(500)
            );
        }

        #[test]
        fn multiple_borrowers_independent() {
            let mut deps = mock_dependencies();
            setup_allowlist(&mut deps, &["uusd"]);

            deposit_collateral(deps.as_mut(), &alice(), "uusd", Uint128::new(300)).unwrap();
            deposit_collateral(deps.as_mut(), &bob(), "uusd", Uint128::new(700)).unwrap();

            assert_eq!(
                weighted_collateral_total(deps.as_ref(), &alice()).unwrap(),
                Uint128::new(300)
            );
            assert_eq!(
                weighted_collateral_total(deps.as_ref(), &bob()).unwrap(),
                Uint128::new(700)
            );
        }
    }
}
