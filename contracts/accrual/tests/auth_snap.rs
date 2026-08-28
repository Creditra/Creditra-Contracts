// SPDX-License-Identifier: MIT
#![cfg(test)]

//! Per-entrypoint authorization snapshots for the accrual (v7) subsystem.
//!
//! # What
//!
//! Verifies the exact authorization footprint of every state-mutating
//! entrypoint in the accrual surface.  Each test:
//!
//! 1. Deploys the [`Credit`] contract and initializes the admin.
//! 2. Enables auth mocking via `env.mock_all_auths()`.
//! 3. Invokes a single entrypoint with minimal valid arguments.
//! 4. Snapshots `env.auths()` and asserts the exact `(address,
//!    contract_address, function_symbol, arguments)` tuple.
//!
//! This is the CI stability guard for the authorization matrix documented in
//! `docs/threat-model.md`: any accidental auth-addition or auth-removal
//! change will break these tests on the next merge, surfacing the
//! regression before it reaches production.
//!
//! # Accrual entrypoint matrix (v7)
//!
//! | Entrypoint | Auth |
//! |---|---|
//! | `accrue_batch` | **none** — public keeper hook, anyone may materialize interest. |
//! | `update_risk_parameters` | admin `require_auth`. |
//! | `self_suspend_credit_line` | borrower `require_auth`. |
//! | `suspend_credit_line` | admin `require_auth`. |
//! | `set_grace_period_config` | admin `require_auth`. |
//! | `set_late_fee_config` | admin `require_auth`. |
//! | `set_late_fee_flat` | admin `require_auth`. |
//! | `set_penalty_surcharge_bps` | admin `require_auth`. |
//! | `set_repayment_schedule` | admin `require_auth`. |
//! | `set_accrual_admin_cooldown` | admin `require_auth`. |
//! | `forgive_debt` | admin `require_auth`. |
//! | `default_credit_line` | admin `require_auth`. |
//! | `close_credit_line` | closer `require_auth` (admin or third party). |
//! | `reinstate_credit_line` | admin `require_auth`. |
//!
//! # See also
//!
//! - [`creditra_credit::auth::require_admin_auth`] — the admin-gating primitive.
//! - `docs/threat-model.md` — the normative authorization matrix.

extern crate std;
use creditra_credit::penalties::{AprFeeConfig, FlatFeeConfig};
use creditra_credit::types::{CreditStatus, GraceWaiverMode, LateFeeConfig};
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger},
    token, Address, Env, IntoVal, Symbol, Vec,
};

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Deploy Credit, initialize admin, register a SAC liquidity token and
/// mint enough to the contract to cover any draw/repay paths that need a
/// token client.  Returns the env, contract id and admin address.
fn setup() -> (Env, Address, Address) {
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
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &1_000_000_i128);

    (env, contract_id, admin)
}

/// Helper: open a credit line and draw a small amount so the borrower has
/// utilization (accrual state).  Returns the new borrower address.
fn open_line_with_utilization(env: &Env, client: &CreditClient) -> Address {
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &50_000_i128, &500_u32, &50_u32);
    env.ledger().set_timestamp(1);
    client.draw_credit(&borrower, &10_000_i128);
    // Clear auths from setup calls so each test snapshot is pristine.
    env.auths();
    borrower
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 1 — accrue_batch (public keeper — NO AUTH)
// ═══════════════════════════════════════════════════════════════════════════

/// `accrue_batch` is a permissionless keeper hook: anyone may materialize
/// interest for a set of borrowers.  The entrypoint performs **no**
/// address-based authorization and the host records zero auths.
#[test]
fn auth_snap_accrue_batch_no_auth_required() {
    let (env, contract_id, _admin) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = open_line_with_utilization(&env, &client);

    env.ledger().set_timestamp(1_000_000);
    let mut borrowers = Vec::new(&env);
    borrowers.push_back(borrower);
    client.accrue_batch(&borrowers);

    let auths = env.auths();
    assert!(
        auths.is_empty(),
        "accrue_batch must require NO authorization (public keeper hook). Got {auths:?}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 2 — Admin-only accrual config entrypoints
// ═══════════════════════════════════════════════════════════════════════════

/// `set_grace_period_config` requires exactly one admin auth with the full
/// parameter tuple (grace_period_seconds, waiver_mode, reduced_rate_bps).
#[test]
fn auth_snap_set_grace_period_config_admin_only() {
    let (env, contract_id, admin) = setup();
    let client = CreditClient::new(&env, &contract_id);

    let grace_secs: u64 = 86_400;
    let mode = GraceWaiverMode::ReducedRate;
    let reduced_bps: u32 = 100;
    client.set_grace_period_config(&grace_secs, &mode, &reduced_bps);

    let auths = env.auths();
    assert_eq!(
        auths,
        std::vec![(
            admin.clone(),
            contract_id.clone(),
            Symbol::new(&env, "set_grace_period_config"),
            (grace_secs, mode, reduced_bps).into_val(&env)
        )],
        "set_grace_period_config auth snapshot mismatch"
    );
}

/// `set_late_fee_config` (admin) with `Some(AprBased(...))` payload.
#[test]
fn auth_snap_set_late_fee_config_admin_only() {
    let (env, contract_id, admin) = setup();
    let client = CreditClient::new(&env, &contract_id);

    let cfg = LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 500 });
    client.set_late_fee_config(&Some(cfg.clone()));

    let auths = env.auths();
    assert_eq!(
        auths,
        std::vec![(
            admin.clone(),
            contract_id.clone(),
            Symbol::new(&env, "set_late_fee_config"),
            (Some(cfg),).into_val(&env)
        )],
        "set_late_fee_config auth snapshot mismatch"
    );
}

/// `set_late_fee_flat` — admin auth snapshot.
#[test]
fn auth_snap_set_late_fee_flat_admin_only() {
    let (env, contract_id, admin) = setup();
    let client = CreditClient::new(&env, &contract_id);

    let flat_fee: i128 = 10_000;
    client.set_late_fee_flat(&flat_fee);

    let auths = env.auths();
    assert_eq!(
        auths,
        std::vec![(
            admin.clone(),
            contract_id.clone(),
            Symbol::new(&env, "set_late_fee_flat"),
            (flat_fee,).into_val(&env)
        )],
        "set_late_fee_flat auth snapshot mismatch"
    );
}

/// `set_penalty_surcharge_bps` — admin auth snapshot.
#[test]
fn auth_snap_set_penalty_surcharge_admin_only() {
    let (env, contract_id, admin) = setup();
    let client = CreditClient::new(&env, &contract_id);

    let bps: u32 = 2_000;
    client.set_penalty_surcharge_bps(&bps);

    let auths = env.auths();
    assert_eq!(
        auths,
        std::vec![(
            admin.clone(),
            contract_id.clone(),
            Symbol::new(&env, "set_penalty_surcharge_bps"),
            (bps,).into_val(&env)
        )],
        "set_penalty_surcharge_bps auth snapshot mismatch"
    );
}

/// `set_accrual_admin_cooldown` — admin auth snapshot.
#[test]
fn auth_snap_set_accrual_admin_cooldown_admin_only() {
    let (env, contract_id, admin) = setup();
    let client = CreditClient::new(&env, &contract_id);

    let seconds: u64 = 3600;
    client.set_accrual_admin_cooldown(&seconds);

    let auths = env.auths();
    assert_eq!(
        auths,
        std::vec![(
            admin.clone(),
            contract_id.clone(),
            Symbol::new(&env, "set_accrual_admin_cooldown"),
            (seconds,).into_val(&env)
        )],
        "set_accrual_admin_cooldown auth snapshot mismatch"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 3 — Per-borrower accrual admin mutations
// ═══════════════════════════════════════════════════════════════════════════

/// `update_risk_parameters` — admin auth snapshot with the four-parameter
/// tuple (borrower, credit_limit, interest_rate_bps, risk_score).
#[test]
fn auth_snap_update_risk_parameters_admin_only() {
    let (env, contract_id, admin) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = open_line_with_utilization(&env, &client);

    let credit_limit: i128 = 75_000;
    let rate_bps: u32 = 700;
    let score: u32 = 75;
    client.update_risk_parameters(&borrower, &credit_limit, &rate_bps, &score);

    let auths = env.auths();
    assert_eq!(
        auths,
        std::vec![(
            admin.clone(),
            contract_id.clone(),
            Symbol::new(&env, "update_risk_parameters"),
            (borrower.clone(), credit_limit, rate_bps, score).into_val(&env)
        )],
        "update_risk_parameters auth snapshot mismatch"
    );
}

/// `suspend_credit_line` — admin auth snapshot (uses accrual admin cooldown).
#[test]
fn auth_snap_suspend_credit_line_admin_only() {
    let (env, contract_id, admin) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = open_line_with_utilization(&env, &client);

    client.suspend_credit_line(&borrower);

    let auths = env.auths();
    assert_eq!(
        auths,
        std::vec![(
            admin.clone(),
            contract_id.clone(),
            Symbol::new(&env, "suspend_credit_line"),
            (borrower.clone(),).into_val(&env)
        )],
        "suspend_credit_line auth snapshot mismatch"
    );
}

/// `self_suspend_credit_line` — **borrower** auth snapshot (NOT admin).
///
/// This is the only accrual lifecycle entrypoint that authorizes the
/// *borrower* rather than the admin — borrowers must always be able to
/// self-suspend even when they cannot reach the admin.
#[test]
fn auth_snap_self_suspend_credit_line_borrower_only() {
    let (env, contract_id, _admin) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = open_line_with_utilization(&env, &client);

    client.self_suspend_credit_line(&borrower);

    let auths = env.auths();
    assert_eq!(
        auths,
        std::vec![(
            borrower.clone(),
            contract_id.clone(),
            Symbol::new(&env, "self_suspend_credit_line"),
            (borrower.clone(),).into_val(&env)
        )],
        "self_suspend_credit_line must use borrower auth, not admin"
    );
}

/// `default_credit_line` — admin auth snapshot.
#[test]
fn auth_snap_default_credit_line_admin_only() {
    let (env, contract_id, admin) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = open_line_with_utilization(&env, &client);

    client.default_credit_line(&borrower);

    let auths = env.auths();
    assert_eq!(
        auths,
        std::vec![(
            admin.clone(),
            contract_id.clone(),
            Symbol::new(&env, "default_credit_line"),
            (borrower.clone(),).into_val(&env)
        )],
        "default_credit_line auth snapshot mismatch"
    );
}

/// `forgive_debt` — admin auth snapshot with (borrower, amount).
#[test]
fn auth_snap_forgive_debt_admin_only() {
    let (env, contract_id, admin) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = open_line_with_utilization(&env, &client);

    let amount: i128 = 1_000;
    client.forgive_debt(&borrower, &amount);

    let auths = env.auths();
    assert_eq!(
        auths,
        std::vec![(
            admin.clone(),
            contract_id.clone(),
            Symbol::new(&env, "forgive_debt"),
            (borrower.clone(), amount).into_val(&env)
        )],
        "forgive_debt auth snapshot mismatch"
    );
}

/// `reinstate_credit_line` — admin auth snapshot with (borrower, target_status).
#[test]
fn auth_snap_reinstate_credit_line_admin_only() {
    let (env, contract_id, admin) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = open_line_with_utilization(&env, &client);
    client.default_credit_line(&borrower);
    let _ = env.auths(); // clear default auths

    let target = CreditStatus::Active;
    client.reinstate_credit_line(&borrower, &target);

    let auths = env.auths();
    assert_eq!(
        auths,
        std::vec![(
            admin.clone(),
            contract_id.clone(),
            Symbol::new(&env, "reinstate_credit_line"),
            (borrower.clone(), target).into_val(&env)
        )],
        "reinstate_credit_line auth snapshot mismatch"
    );
}

/// `close_credit_line` — closer auth (admin used as closer in this test).
#[test]
fn auth_snap_close_credit_line_closer_auth() {
    let (env, contract_id, admin) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &50_000_i128, &500_u32, &50_u32);
    let _ = env.auths(); // clear open auths

    let closer = admin.clone();
    client.close_credit_line(&borrower, &closer);

    let auths = env.auths();
    assert_eq!(
        auths,
        std::vec![(
            closer.clone(),
            contract_id.clone(),
            Symbol::new(&env, "close_credit_line"),
            (borrower.clone(), closer.clone()).into_val(&env)
        )],
        "close_credit_line must use closer auth (not implicitly admin)"
    );
}

/// `set_repayment_schedule` — admin auth snapshot.
#[test]
fn auth_snap_set_repayment_schedule_admin_only() {
    let (env, contract_id, admin) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = open_line_with_utilization(&env, &client);

    let amount_per_period: i128 = 5_000;
    let period_seconds: u64 = 2_592_000;
    let first_due_ts: u64 = 10_000_000;
    client.set_repayment_schedule(
        &borrower,
        &amount_per_period,
        &period_seconds,
        &first_due_ts,
    );

    let auths = env.auths();
    assert_eq!(
        auths,
        std::vec![(
            admin.clone(),
            contract_id.clone(),
            Symbol::new(&env, "set_repayment_schedule"),
            (
                borrower.clone(),
                amount_per_period,
                period_seconds,
                first_due_ts
            )
                .into_val(&env)
        )],
        "set_repayment_schedule auth snapshot mismatch"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 4 — Compound checks
// ═══════════════════════════════════════════════════════════════════════════

/// `close_credit_line` with a non-admin third-party closer confirms that
/// `closer.require_auth()` — the closer is the *third party*, NOT the admin.
#[test]
fn auth_snap_close_credit_line_third_party_closer_not_admin() {
    let (env, contract_id, admin) = setup();
    let client = CreditClient::new(&env, &contract_id);
    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &50_000_i128, &500_u32, &50_u32);
    let _ = env.auths();

    let third_party_closer = Address::generate(&env);
    client.close_credit_line(&borrower, &third_party_closer);

    let auths = env.auths();
    assert_eq!(auths.len(), 1, "exactly one auth (the closer)");
    let (auth_addr, _contract, sym, _args) = &auths[0];
    assert_eq!(
        auth_addr, &third_party_closer,
        "closer must be third party, not admin"
    );
    assert_ne!(
        auth_addr, &admin,
        "admin must NOT be required for third-party close"
    );
    assert_eq!(sym, &Symbol::new(&env, "close_credit_line"));
}

/// Sequential admin calls produce *exactly* one admin auth per call — no
/// cross-call bleed-through and no omitted auths.
#[test]
fn auth_snap_sequential_admin_calls_each_require_one_admin_auth() {
    let (env, contract_id, admin) = setup();
    let client = CreditClient::new(&env, &contract_id);

    client.set_penalty_surcharge_bps(&100_u32);
    let auths1 = env.auths();
    assert_eq!(auths1.len(), 1, "first call: exactly one auth");
    assert_eq!(auths1[0].0, admin);

    client.set_accrual_admin_cooldown(&600_u64);
    let auths2 = env.auths();
    assert_eq!(
        auths2.len(),
        1,
        "second call: exactly one auth (no bleed-through)"
    );
    assert_eq!(auths2[0].0, admin);
}
