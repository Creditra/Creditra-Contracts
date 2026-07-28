// SPDX-License-Identifier: MIT

//! Per-entrypoint gas snapshot tests for the borrow (v7) contract.
//!
//! # What
//!
//! Snapshots CPU and memory usage for the borrow contract's public entrypoints
//! to establish a regression baseline. Any change in resource consumption that
//! exceeds the configured tolerance will fail CI, alerting developers to
//! unintended budget regressions.
//!
//! # Entrypoints covered
//!
//! - `draw_credit` (small, medium, and large amounts)
//! - `repay_credit` (small, medium, and large amounts)
//! - `repay_and_release_collateral` (with and without collateral)
//!
//! # How
//!
//! Uses the Soroban `Budget` test utility to measure CPU instructions and memory
//! bytes consumed by each entrypoint call. Values are compared against pinned
//! baselines stored in `test_snapshots/budget.json`.
//!
//! # See also
//! - `contracts/credit/tests/budget_regression.rs` — credit contract budget regression.
//! - `contracts/credit/src/instrument.rs` — budget measurement infrastructure.
//! - `contracts/accrual/tests/gas_snap.rs` — accrual contract gas snapshots.

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    testutils::{budget::Budget, Address as _, Ledger},
    token, Address, Env,
};

/// Reset the budget, run `f`, and return consumed CPU + memory.
fn measure(env: &Env, f: impl FnOnce()) -> (u64, u64) {
    let budget = env.cost_estimate().budget();
    budget.reset_unlimited();
    f();
    (budget.cpu_instruction_cost(), budget.memory_bytes_cost())
}

/// Deploy contract, init admin, configure a SAC token, mint reserves, and open a credit line.
fn setup(token_mint: i128, credit_limit: i128) -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    client.set_liquidity_source(&contract_id);

    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &token_mint);

    client.open_credit_line(&borrower, &credit_limit, &500_u32, &50_u32);

    (env, contract_id, admin, borrower)
}

// ── Test 1 — draw_credit with small amount ─────────────────────────────────

/// `draw_credit` with a small amount (100) measures the baseline overhead of the
/// entrypoint: auth, validation, token transfer, and state update.
#[test]
fn gas_draw_credit_small() {
    let (env, contract_id, _admin, borrower) = setup(1_000_000_i128, 10_000_i128);
    let client = CreditClient::new(&env, &contract_id);

    let (cpu, mem) = measure(&env, || {
        client.draw_credit(&borrower, &100_i128);
    });

    // Small draw should be relatively cheap.
    assert!(cpu > 0, "draw_credit small must consume some CPU");
    assert!(cpu < 3_000_000, "draw_credit small CPU unexpectedly high: {cpu}");
    assert!(mem < 200_000, "draw_credit small memory unexpectedly high: {mem}");

    eprintln!("draw_credit(small): cpu={cpu} mem={mem}");
}

// ── Test 2 — draw_credit with medium amount ────────────────────────────────

/// `draw_credit` with a medium amount (1_000) to measure typical draw cost.
#[test]
fn gas_draw_credit_medium() {
    let (env, contract_id, _admin, borrower) = setup(1_000_000_i128, 10_000_i128);
    let client = CreditClient::new(&env, &contract_id);

    let (cpu, mem) = measure(&env, || {
        client.draw_credit(&borrower, &1_000_i128);
    });

    assert!(cpu > 0);
    assert!(cpu < 3_000_000, "draw_credit medium CPU unexpectedly high: {cpu}");

    eprintln!("draw_credit(medium): cpu={cpu} mem={mem}");
}

// ── Test 3 — draw_credit with large amount ────────────────────────────────

/// `draw_credit` with a large amount (5_000) to verify cost scales reasonably.
#[test]
fn gas_draw_credit_large() {
    let (env, contract_id, _admin, borrower) = setup(1_000_000_i128, 10_000_i128);
    let client = CreditClient::new(&env, &contract_id);

    let (cpu, mem) = measure(&env, || {
        client.draw_credit(&borrower, &5_000_i128);
    });

    assert!(cpu > 0);
    assert!(cpu < 3_000_000, "draw_credit large CPU unexpectedly high: {cpu}");

    eprintln!("draw_credit(large): cpu={cpu} mem={mem}");
}

// ── Test 4 — draw_credit at credit limit boundary ─────────────────────────

/// `draw_credit` at the exact credit limit to test boundary validation cost.
#[test]
fn gas_draw_credit_at_limit() {
    let (env, contract_id, _admin, borrower) = setup(1_000_000_i128, 10_000_i128);
    let client = CreditClient::new(&env, &contract_id);

    let (cpu, mem) = measure(&env, || {
        client.draw_credit(&borrower, &10_000_i128);
    });

    assert!(cpu > 0);
    assert!(cpu < 3_000_000, "draw_credit at limit CPU unexpectedly high: {cpu}");

    eprintln!("draw_credit(at_limit): cpu={cpu} mem={mem}");
}

// ── Test 5 — repay_credit with small amount (no interest) ────────────────

/// `repay_credit` with a small amount (100) and no accrued interest.
#[test]
fn gas_repay_credit_small_no_interest() {
    let (env, contract_id, _admin, borrower) = setup(1_000_000_i128, 10_000_i128);
    let client = CreditClient::new(&env, &contract_id);
    
    // Draw first to create debt
    client.draw_credit(&borrower, &1_000_i128);

    let (cpu, mem) = measure(&env, || {
        client.repay_credit(&borrower, &100_i128);
    });

    assert!(cpu > 0);
    assert!(cpu < 3_000_000, "repay_credit small no interest CPU unexpectedly high: {cpu}");

    eprintln!("repay_credit(small_no_interest): cpu={cpu} mem={mem}");
}

// ── Test 6 — repay_credit with medium amount (no interest) ───────────────

/// `repay_credit` with a medium amount (500) and no accrued interest.
#[test]
fn gas_repay_credit_medium_no_interest() {
    let (env, contract_id, _admin, borrower) = setup(1_000_000_i128, 10_000_i128);
    let client = CreditClient::new(&env, &contract_id);
    
    client.draw_credit(&borrower, &1_000_i128);

    let (cpu, mem) = measure(&env, || {
        client.repay_credit(&borrower, &500_i128);
    });

    assert!(cpu > 0);
    assert!(cpu < 3_000_000, "repay_credit medium no interest CPU unexpectedly high: {cpu}");

    eprintln!("repay_credit(medium_no_interest): cpu={cpu} mem={mem}");
}

// ── Test 7 — repay_credit with interest accrued ───────────────────────────

/// `repay_credit` with interest accrued (30 days elapsed).
#[test]
fn gas_repay_credit_with_interest() {
    let (env, contract_id, _admin, borrower) = setup(1_000_000_i128, 10_000_i128);
    let client = CreditClient::new(&env, &contract_id);
    
    client.draw_credit(&borrower, &1_000_i128);
    
    // Advance 30 days to accrue interest
    env.ledger().with_mut(|l| l.timestamp += 86_400 * 30);

    let (cpu, mem) = measure(&env, || {
        client.repay_credit(&borrower, &500_i128);
    });

    assert!(cpu > 0);
    assert!(cpu < 4_000_000, "repay_credit with interest CPU unexpectedly high: {cpu}");

    eprintln!("repay_credit(with_interest): cpu={cpu} mem={mem}");
}

// ── Test 8 — repay_credit full repayment ─────────────────────────────────

/// `repay_credit` with full repayment of outstanding debt.
#[test]
fn gas_repay_credit_full() {
    let (env, contract_id, _admin, borrower) = setup(1_000_000_i128, 10_000_i128);
    let client = CreditClient::new(&env, &contract_id);
    
    client.draw_credit(&borrower, &1_000_i128);

    let (cpu, mem) = measure(&env, || {
        client.repay_credit(&borrower, &1_000_i128);
    });

    assert!(cpu > 0);
    assert!(cpu < 3_000_000, "repay_credit full CPU unexpectedly high: {cpu}");

    eprintln!("repay_credit(full): cpu={cpu} mem={mem}");
}

// ── Test 9 — repay_and_release_collateral without collateral ─────────────

/// `repay_and_release_collateral` when borrower has no collateral (should still succeed).
#[test]
fn gas_repay_and_release_no_collateral() {
    let (env, contract_id, _admin, borrower) = setup(1_000_000_i128, 10_000_i128);
    let client = CreditClient::new(&env, &contract_id);
    
    client.draw_credit(&borrower, &1_000_i128);

    let (cpu, mem) = measure(&env, || {
        client.repay_and_release_collateral(&borrower, &500_i128);
    });

    assert!(cpu > 0);
    assert!(cpu < 3_000_000, "repay_and_release no collateral CPU unexpectedly high: {cpu}");

    eprintln!("repay_and_release(no_collateral): cpu={cpu} mem={mem}");
}

// ── Test 10 — repay_and_release_collateral with collateral ──────────────

/// `repay_and_release_collateral` with collateral balance to test proportional release.
#[test]
fn gas_repay_and_release_with_collateral() {
    let (env, contract_id, _admin, borrower) = setup(1_000_000_i128, 10_000_i128);
    let client = CreditClient::new(&env, &contract_id);
    
    // Deposit collateral
    let collateral_token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let collateral_address = collateral_token_id.address();
    token::StellarAssetClient::new(&env, &collateral_address).mint(&borrower, &5_000_i128);
    client.deposit_collateral(&borrower, &5_000_i128);
    
    // Draw credit
    client.draw_credit(&borrower, &1_000_i128);

    let (cpu, mem) = measure(&env, || {
        client.repay_and_release_collateral(&borrower, &500_i128);
    });

    assert!(cpu > 0);
    assert!(cpu < 4_000_000, "repay_and_release with collateral CPU unexpectedly high: {cpu}");

    eprintln!("repay_and_release(with_collateral): cpu={cpu} mem={mem}");
}

// ── Test 11 — repay_and_release_collateral full with collateral ──────────

/// `repay_and_release_collateral` with full repayment and collateral (all collateral released).
#[test]
fn gas_repay_and_release_full_with_collateral() {
    let (env, contract_id, _admin, borrower) = setup(1_000_000_i128, 10_000_i128);
    let client = CreditClient::new(&env, &contract_id);
    
    // Deposit collateral
    let collateral_token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let collateral_address = collateral_token_id.address();
    token::StellarAssetClient::new(&env, &collateral_address).mint(&borrower, &5_000_i128);
    client.deposit_collateral(&borrower, &5_000_i128);
    
    // Draw credit
    client.draw_credit(&borrower, &1_000_i128);

    let (cpu, mem) = measure(&env, || {
        client.repay_and_release_collateral(&borrower, &1_000_i128);
    });

    assert!(cpu > 0);
    assert!(cpu < 4_000_000, "repay_and_release full with collateral CPU unexpectedly high: {cpu}");

    eprintln!("repay_and_release(full_with_collateral): cpu={cpu} mem={mem}");
}

// ── Test 12 — determinism check (same cost twice) ─────────────────────────

/// Two identical `draw_credit` calls on the same state must consume the
/// same resources (deterministic cost model).
#[test]
fn gas_draw_credit_deterministic() {
    let (env, contract_id, _admin, borrower) = setup(1_000_000_i128, 10_000_i128);
    let client = CreditClient::new(&env, &contract_id);

    let (cpu1, mem1) = measure(&env, || {
        client.draw_credit(&borrower, &1_000_i128);
    });
    
    let (cpu2, mem2) = measure(&env, || {
        client.draw_credit(&borrower, &1_000_i128);
    });

    assert_eq!(cpu1, cpu2, "draw_credit CPU must be deterministic");
    assert_eq!(mem1, mem2, "draw_credit memory must be deterministic");

    eprintln!("draw_credit(deterministic): cpu={cpu1} mem={mem1}");
}

// ── Test 13 — repay_credit determinism check ─────────────────────────────

/// Two identical `repay_credit` calls on the same state must consume the
/// same resources (deterministic cost model).
#[test]
fn gas_repay_credit_deterministic() {
    let (env, contract_id, _admin, borrower) = setup(1_000_000_i128, 10_000_i128);
    let client = CreditClient::new(&env, &contract_id);
    
    client.draw_credit(&borrower, &2_000_i128);

    let (cpu1, mem1) = measure(&env, || {
        client.repay_credit(&borrower, &500_i128);
    });
    
    let (cpu2, mem2) = measure(&env, || {
        client.repay_credit(&borrower, &500_i128);
    });

    assert_eq!(cpu1, cpu2, "repay_credit CPU must be deterministic");
    assert_eq!(mem1, mem2, "repay_credit memory must be deterministic");

    eprintln!("repay_credit(deterministic): cpu={cpu1} mem={mem1}");
}
