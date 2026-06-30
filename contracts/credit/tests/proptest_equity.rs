// SPDX-License-Identifier: MIT

//! Property test: protocol equity remains non-negative after liquidation settlement.
//!
//! # Invariant
//!
//! After [`settle_default_liquidation`]:
//!
//! 1. **Borrower utilized_amount >= 0** — the settlement subtraction never
//!    makes the borrower's debt negative (enforced by `checked_sub`).
//! 2. **TotalUtilized accounting consistency** — the change in the global
//!    `total_utilized` accumulator exactly matches the change in the
//!    borrower's individual `utilized_amount`.
//! 3. **Protocol equity >= 0** — defined as
//!    `total_collateral + treasury_balance - total_utilized`, the protocol's
//!    net asset position must not go negative after a settlement.
//!
//! # Why
//!
//! During liquidation, the contract:
//! 1. Capitalizes pending interest via `apply_accrual` (which can increase
//!    `utilized_amount`).
//! 2. Subtracts `recovered_amount` from the borrower's `utilized_amount`.
//! 3. Adjusts the global `total_utilized` accumulator by the net delta.
//!
//! A bug in any of these steps — an off-by-one in `persist_credit_line`, an
//! overflow in `adjust_total_utilized`, or a missed accrual — would break
//! the accounting invariants.
//!
//! # Strategy
//!
//! Random valid `(credit_limit, utilized_amount, close_factor_bps, recovery_pct)`
//! tuples drive `settle_default_liquidation` and the three invariants are
//! checked after every call.

use proptest::prelude::*;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{token, Address, Env, Symbol};

use creditra_credit::{Credit, CreditClient};

/// Deploy a credit contract, open a line for one borrower, draw, then default.
///
/// The borrower is minted enough tokens and approval so that a subsequent
/// `deposit_collateral` call can succeed in the proptest body if needed.
fn setup_defaulted_borrower(
    env: &Env,
    credit_limit: i128,
    utilized_amount: i128,
) -> (CreditClient<'_>, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    client.set_liquidity_source(&contract_id);

    let sac = token::StellarAssetClient::new(env, &token_address);
    sac.mint(&contract_id, &(credit_limit * 2));
    sac.mint(&borrower, &(credit_limit * 2));

    client.open_credit_line(&borrower, &credit_limit, &500_u32, &50_u32);
    client.deposit_collateral(&borrower, &(credit_limit * 2));
    client.draw_credit(&borrower, &utilized_amount);
    client.default_credit_line(&borrower);

    (client, borrower)
}

/// Compute protocol equity from a summary.
fn protocol_equity(summary: &creditra_credit::types::ProtocolSummary) -> i128 {
    summary
        .total_collateral
        .checked_add(summary.treasury_balance)
        .and_then(|v| v.checked_sub(summary.total_utilized))
        .unwrap_or(i128::MIN)
}

proptest! {
    /// Verifies the three equity-related invariants after liquidation.
    ///
    /// 1. `recovered_amount > 0` (the contract enforces this).
    /// 2. `close_factor_bps` in `[1000, 10000]` — we use 1000 as a minimum so
    ///    that `max_recovery >= utilized / 10`, ensuring non-trivial recovery
    ///    amounts in most cases.
    /// 3. After settlement, the borrower's `utilized_amount` stays `>= 0`.
    /// 4. The global `total_utilized` delta matches the per-borrower delta.
    /// 5. `equity >= 0` after every successful settlement.
    #[test]
    fn prop_equity_non_negative_after_liquidation(
        credit_limit in 10_000i128..=100_000i128,
        utilized in 1_000i128..=50_000i128,
        close_factor_bps in 1_000u32..=10_000u32,
        recovery_pct in 10u32..=100u32,
    ) {
        // ── Pre-condition filtering ────────────────────────────────────────
        if utilized > credit_limit {
            return Ok(());
        }

        let max_recovery = utilized * close_factor_bps as i128 / 10_000;
        if max_recovery < 1 {
            return Ok(());
        }

        let recovered_amount = max_recovery * recovery_pct as i128 / 100;
        if recovered_amount < 1 {
            return Ok(());
        }

        // ── Setup ──────────────────────────────────────────────────────────
        let env = Env::default();
        env.ledger().set_timestamp(100_000);
        let (client, borrower) = setup_defaulted_borrower(&env, credit_limit, utilized);

        // ── Snapshot before ────────────────────────────────────────────────
        let summary_before = client.get_protocol_summary();
        let line_before = client.get_credit_line(&borrower).unwrap();
        let equity_before = protocol_equity(&summary_before);

        // ── Execute liquidation ────────────────────────────────────────────
        let settlement_id = Symbol::new(&env, "prop_liq");
        client.settle_default_liquidation(
            &borrower,
            &recovered_amount,
            &settlement_id,
            &close_factor_bps,
            &None,
        );

        // ── Snapshot after ─────────────────────────────────────────────────
        let summary_after = client.get_protocol_summary();
        let line_after = client.get_credit_line(&borrower).unwrap();
        let equity_after = protocol_equity(&summary_after);

        // ── Invariant 1: borrower utilized_amount never negative ───────────
        prop_assert!(
            line_after.utilized_amount >= 0,
            "borrower utilized_amount dropped below zero:\n\
             before={}, after={}, recovered={}, cf={}",
            line_before.utilized_amount,
            line_after.utilized_amount,
            recovered_amount,
            close_factor_bps,
        );

        // ── Invariant 2: total_utilized consistency ────────────────────────
        // The global accumulator must change by exactly the same amount as
        // the borrower's individual utilized_amount.
        let borrower_delta = line_after
            .utilized_amount
            .checked_sub(line_before.utilized_amount)
            .unwrap_or(i128::MIN);
        let total_delta = summary_after
            .total_utilized
            .checked_sub(summary_before.total_utilized)
            .unwrap_or(i128::MIN);

        prop_assert_eq!(
            total_delta, borrower_delta,
            "total_utilized delta ({}) != borrower utilized delta ({})",
            total_delta, borrower_delta,
        );

        // ── Invariant 3: protocol equity never goes negative ───────────────
        prop_assert!(
            equity_after >= 0,
            "protocol equity went negative after liquidation:\n\
             equity_before={}, equity_after={}\n\
             collateral={}, treasury={}, utilized={}",
            equity_before,
            equity_after,
            summary_after.total_collateral,
            summary_after.treasury_balance,
            summary_after.total_utilized,
        );

        // ── Derived: equity is non-decreasing ──────────────────────────────
        // Liquidation reduces total_utilized (or keeps it the same), so
        // equity should never decrease (the collateral and treasury accounts
        // are not touched by settle_default_liquidation).
        prop_assert!(
            equity_after >= equity_before,
            "equity decreased during liquidation: before={}, after={}",
            equity_before,
            equity_after,
        );
    }
}

// ── Edge-case unit tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod edge_cases {
    use super::*;
    use creditra_credit::types::CreditStatus;

    /// Recovered amount must be > 0 (the contract enforces this).
    #[test]
    fn zero_recovery_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, borrower) = setup_defaulted_borrower(&env, 10_000, 1_000);
        let sid = Symbol::new(&env, "zero_rec");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.settle_default_liquidation(&borrower, &0_i128, &sid, &10_000_u32, &None);
        }));
        assert!(result.is_err(), "zero recovery must panic");
    }

    /// Full liquidation (close_factor = 10000, recovered == utilized) closes
    /// the line and reduces total_utilized to zero.
    #[test]
    fn full_liquidation_zeroes_debt_and_closes() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, borrower) = setup_defaulted_borrower(&env, 10_000, 3_000);

        let summary_before = client.get_protocol_summary();

        let sid = Symbol::new(&env, "full_liq");
        client.settle_default_liquidation(&borrower, &3_000_i128, &sid, &10_000_u32, &None);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.utilized_amount, 0);
        assert_eq!(line.status, CreditStatus::Closed);

        // With only one borrower, total_utilized should be 0.
        let summary_after = client.get_protocol_summary();
        assert_eq!(summary_after.total_utilized, 0);

        let equity = protocol_equity(&summary_after);
        assert!(equity >= 0, "equity went negative: {}", equity);

        // total_utilized delta should equal borrower delta
        let borrower_delta = 0i128 - 3_000i128; // after - before
        let total_delta = summary_after.total_utilized - summary_before.total_utilized;
        assert_eq!(
            total_delta, borrower_delta,
            "total_utilized mismatch after full liquidation"
        );
    }

    /// Partial liquidation (close_factor < 10000) leaves the line in
    /// Defaulted with reduced utilized_amount.
    #[test]
    fn partial_liquidation_reduces_debt_keeps_defaulted() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, borrower) = setup_defaulted_borrower(&env, 10_000, 2_000);

        let sid = Symbol::new(&env, "part_liq");
        // close_factor = 5000 bps (50%), recovered = 500
        // max_recovery = 2000 * 5000 / 10000 = 1000
        // we recover 500
        client.settle_default_liquidation(&borrower, &500_i128, &sid, &5_000_u32, &None);

        let line = client.get_credit_line(&borrower).unwrap();
        assert!(line.utilized_amount > 0);
        assert_eq!(line.status, CreditStatus::Defaulted);
    }

    /// Replay protection: the same settlement_id cannot be used twice.
    #[test]
    fn replay_using_same_settlement_id_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, borrower) = setup_defaulted_borrower(&env, 10_000, 1_000);

        let sid = Symbol::new(&env, "replay_id");
        client.settle_default_liquidation(&borrower, &500_i128, &sid, &10_000_u32, &None);

        let replay = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.settle_default_liquidation(&borrower, &100_i128, &sid, &10_000_u32, &None);
        }));
        assert!(replay.is_err(), "replay must panic");
    }
}
