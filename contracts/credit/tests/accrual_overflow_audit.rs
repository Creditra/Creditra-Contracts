// SPDX-License-Identifier: MIT

//! Overflow audit tests for `apply_accrual` and `compute_interest`.
//!
//! # Coverage
//! - Max principal + max rate (10 000 bps) + long elapsed time does not panic
//!   or silently wrap — it either produces a valid result or reverts with
//!   `ContractError::Overflow` (discriminant 12).
//! - The overflow path is deterministic: the same inputs always produce
//!   `ContractError::Overflow`, never a wrong numeric result.

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token, Address, Env,
};

// ── helpers ───────────────────────────────────────────────────────────────────

/// Deploy the contract, init admin, configure a SAC token as liquidity source,
/// and mint `reserve_amount` tokens into the contract address (the reserve).
fn setup_with_token(reserve_amount: i128) -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    // Register a Stellar Asset Contract to act as the liquidity token.
    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();

    client.set_liquidity_token(&token_address);
    client.set_liquidity_source(&contract_id);

    // Mint reserve tokens into the contract so draws can succeed.
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &reserve_amount);

    (env, contract_id, admin, token_address)
}

// ── Test 1: max principal + max rate + long elapsed time ─────────────────────

/// With a very large principal (1e18) and the maximum rate (10 000 bps = 100%),
/// advancing 1 000 years must not panic or silently wrap.
///
/// `1e18 * 10_000 * (1_000 * 31_536_000)` = `3.15e29`, which is well within
/// `i128::MAX` (~1.7e38), so this case must succeed and return a positive
/// accrued amount.
#[test]
fn max_rate_large_principal_long_elapsed_does_not_panic() {
    let principal: i128 = 1_000_000_000_000_000_000; // 1e18
    let (env, contract_id, _admin, _token) = setup_with_token(principal);
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    // Open line at max rate (10 000 bps).
    client.open_credit_line(&borrower, &principal, &10_000_u32, &50_u32);

    // Draw at t = 1 to establish the accrual checkpoint.
    env.ledger().set_timestamp(1);
    client.draw_credit(&borrower, &principal);

    // Advance 1 000 years.
    let one_thousand_years: u64 = 1_000 * 31_536_000;
    env.ledger().set_timestamp(1 + one_thousand_years);

    // Trigger accrual — must not panic.
    client.update_risk_parameters(&borrower, &i128::MAX, &10_000_u32, &50_u32);

    let line = client.get_credit_line(&borrower).unwrap();

    // Accrued interest must be positive and the line must still be readable.
    assert!(
        line.accrued_interest > 0,
        "expected positive accrued interest, got {}",
        line.accrued_interest
    );
    assert!(
        line.utilized_amount > principal,
        "utilized_amount must have grown from accrual"
    );
}

// ── Test 2: overflow path returns ContractError::Overflow deterministically ───

/// When `utilized * rate_bps` already overflows `i128`, `compute_interest`
/// must return `ContractError::Overflow` (discriminant 12) — never a wrong
/// numeric result and never a bare panic.
///
/// `i128::MAX / 2 * 10_000` overflows `i128`, so any positive elapsed time
/// triggers the overflow path.
#[test]
fn overflow_path_returns_contract_error_overflow_deterministically() {
    // i128::MAX / 2 overflows when multiplied by 10_000.
    let huge_principal: i128 = i128::MAX / 2;
    let (env, contract_id, _admin, _token) = setup_with_token(huge_principal);
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);

    client.open_credit_line(&borrower, &huge_principal, &10_000_u32, &50_u32);

    // Draw at t = 1.
    env.ledger().set_timestamp(1);
    client.draw_credit(&borrower, &huge_principal);

    // Advance time so accrual is triggered.
    env.ledger().set_timestamp(2);

    // Trigger accrual — must revert with ContractError::Overflow (code 12).
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.update_risk_parameters(&borrower, &i128::MAX, &10_000_u32, &50_u32);
    }));

    assert!(
        result.is_err(),
        "expected a revert on overflow, but the call succeeded"
    );

    // The panic payload from Soroban encodes the contract error discriminant.
    // ContractError::Overflow = 12, encoded as "Error(Contract, #12)".
    // We verify the error string contains the discriminant to confirm it is
    // ContractError::Overflow and not some other panic.
    let err = result.unwrap_err();
    let err_str = if let Some(s) = err.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = err.downcast_ref::<&str>() {
        s.to_string()
    } else {
        String::new()
    };

    assert!(
        err_str.contains("#12"),
        "expected ContractError::Overflow (#12) but got: {:?}",
        err_str
    );
}

/// Same overflow scenario repeated a second time with identical inputs must
/// produce the same `ContractError::Overflow` — confirming determinism.
#[test]
fn overflow_path_is_deterministic_same_inputs_same_error() {
    let huge_principal: i128 = i128::MAX / 2;

    for run in 0..2_usize {
        let (env, contract_id, _admin, _token) = setup_with_token(huge_principal);
        let client = CreditClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        client.open_credit_line(&borrower, &huge_principal, &10_000_u32, &50_u32);

        env.ledger().set_timestamp(1);
        client.draw_credit(&borrower, &huge_principal);
        env.ledger().set_timestamp(2);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.update_risk_parameters(&borrower, &i128::MAX, &10_000_u32, &50_u32);
        }));

        assert!(result.is_err(), "must revert on overflow (run {})", run + 1);

        let err = result.unwrap_err();
        let err_str = if let Some(s) = err.downcast_ref::<String>() {
            s.clone()
        } else if let Some(s) = err.downcast_ref::<&str>() {
            s.to_string()
        } else {
            String::new()
        };

        assert!(
            err_str.contains("#12"),
            "run {}: expected #12 but got: {:?}",
            run + 1,
            err_str
        );
    }
}
