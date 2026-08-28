// SPDX-License-Identifier: MIT

//! ContractError stability tests for the query (v7) subsystem.
//!
//! Freezes the [`creditra_credit::types::ContractError`] discriminants and
//! category mappings observable by callers of the credit contract's read-only
//! query entrypoints. See `contracts/query/src/events.rs` for the structured
//! events that accompany these query calls.

use creditra_credit::types::{ContractError, ContractErrorCategory};
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token, Address, Env, Vec,
};

#[test]
fn query_v7_error_discriminants_are_pinned() {
    assert_eq!(ContractError::Overflow as u32, 12);
    assert_eq!(ContractError::CreditLineNotFound as u32, 3);
    assert_eq!(ContractError::CreditLineClosed as u32, 4);
    assert_eq!(ContractError::CreditLineSuspended as u32, 20);
    assert_eq!(ContractError::CreditLineDefaulted as u32, 21);
    assert_eq!(ContractError::CreditLineFrozen as u32, 46);
    assert_eq!(ContractError::Unauthorized as u32, 1);
    assert_eq!(ContractError::NotAdmin as u32, 2);
    assert_eq!(ContractError::AdminNotInitialized as u32, 32);
    assert_eq!(ContractError::InvalidAmount as u32, 5);
    assert_eq!(ContractError::Paused as u32, 18);
    assert_eq!(ContractError::BorrowerBlocked as u32, 16);
    assert_eq!(ContractError::BorrowerFrozen as u32, 40);
    assert_eq!(ContractError::DrawsFrozen as u32, 19);
    assert_eq!(ContractError::MissingLiquidityToken as u32, 22);
    assert_eq!(ContractError::ExposureCapExceeded as u32, 31);
    assert_eq!(ContractError::OverLimit as u32, 6);
    assert_eq!(ContractError::OraclePriceInvalid as u32, 36);
    assert_eq!(ContractError::OracleQuorumNotMet as u32, 50);
    assert_eq!(ContractError::CollateralRatioBelowMinimum as u32, 35);
    assert_eq!(ContractError::Reentrancy as u32, 11);
}

#[test]
fn query_v7_category_mappings_are_pinned() {
    use ContractErrorCategory::*;
    assert_eq!(ContractError::Overflow.category(), Numeric);
    assert_eq!(ContractError::CreditLineNotFound.category(), Misc);
    assert_eq!(ContractError::CreditLineClosed.category(), Lifecycle);
    assert_eq!(ContractError::Paused.category(), Risk);
    assert_eq!(ContractError::BorrowerBlocked.category(), Block);
    assert_eq!(ContractError::CreditLineFrozen.category(), Block);
    assert_eq!(ContractError::ExposureCapExceeded.category(), Liquidity);
    assert_eq!(
        ContractError::CollateralRatioBelowMinimum.category(),
        Collateral
    );
    assert_eq!(ContractError::Reentrancy.category(), Reentrancy);
    assert_eq!(ContractError::OraclePriceInvalid.category(), Oracle);
}

#[test]
fn paginated_over_limit_reverts_with_overflow_code_12() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    client.set_liquidity_source(&contract_id);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.get_credit_lines_paginated(&None, &101_u32);
    }));
    assert!(result.is_err());
    let err = if let Some(s) = result.unwrap_err().downcast_ref::<String>() {
        s.clone()
    } else {
        String::new()
    };
    assert!(err.contains("#12"), "expected Overflow (#12), got: {err:?}");
}
