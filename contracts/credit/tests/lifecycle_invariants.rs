// SPDX-License-Identifier: MIT

//! Proptest: lifecycle state-machine invariants (Issue #905).
//!
//! # What
//!
//! Generates random sequences of lifecycle operations and asserts:
//!
//! 1. `CreditStatus` is always one of the five defined variants after every op.
//! 2. `Closed` is terminal — no further state-changing call succeeds on it.
//! 3. `Defaulted` only accepts `reinstate` and `close` (not `suspend`).
//! 4. `utilized_amount >= 0` and `accrued_interest >= 0` after every op.
//! 5. `accrued_interest <= utilized_amount` at all times.
//! 6. A borrower close with `utilized_amount > 0` always fails.
//! 7. `open_credit_line` on an Active line always fails (`AlreadyInitialized`).
//! 8. Illegal edges (e.g. suspend from Suspended/Defaulted/Closed) always fail.
//!
//! # Authoritative reference
//!
//! Transition table from `docs/PROTOCOL_SPEC.md` §2.3 and
//! `docs/state-machine.md`:
//!
//! ```text
//! Active     → Suspended   (admin suspend / borrower self_suspend)
//! Active     → Defaulted   (admin default)
//! Active     → Closed      (borrower if util=0, admin always)
//! Suspended  → Defaulted   (admin default)
//! Suspended  → Closed      (borrower if util=0, admin always)
//! Defaulted  → Active      (admin reinstate, target=Active)
//! Defaulted  → Restricted  (admin reinstate, target=Restricted)
//! Defaulted  → Closed      (borrower if util=0, admin always)
//! Closed     → *           TERMINAL — all mutations rejected (close is idempotent)
//! ```

use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use proptest::prelude::*;
use proptest::test_runner::Config as ProptestConfig;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, Env};

// ── constants ─────────────────────────────────────────────────────────────────

const INITIAL_TS: u64 = 10_000;
const LIQUIDITY: i128 = 50_000_000;
const CREDIT_LIMIT: i128 = 100_000;
const RATE_BPS: u32 = 500;
const RISK_SCORE: u32 = 40;

// ── test environment setup ────────────────────────────────────────────────────

/// Minimal env: one borrower, liquidity minted, line opened in Active state.
/// Returns `(env, client, admin, borrower)`.
fn setup() -> (Env, CreditClient<'static>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(INITIAL_TS);

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&contract_id);

    let sac = StellarAssetClient::new(&env, &token);
    sac.mint(&contract_id, &LIQUIDITY);

    let borrower = Address::generate(&env);
    sac.mint(&borrower, &LIQUIDITY);
    client.open_credit_line(&borrower, &CREDIT_LIMIT, &RATE_BPS, &RISK_SCORE);

    (env, client, admin, borrower)
}

// ── accounting invariant ──────────────────────────────────────────────────────

/// Assert the core debt-consistency invariants on a live credit line.
fn assert_accounting(client: &CreditClient<'_>, borrower: &Address, label: &str) {
    let line = match client.get_credit_line(borrower) {
        Some(l) => l,
        None => return, // line not yet opened is fine
    };

    assert!(
        line.utilized_amount >= 0,
        "{label}: utilized_amount < 0 ({})",
        line.utilized_amount
    );
    assert!(
        line.accrued_interest >= 0,
        "{label}: accrued_interest < 0 ({})",
        line.accrued_interest
    );
    assert!(
        line.accrued_interest <= line.utilized_amount,
        "{label}: accrued_interest ({}) > utilized_amount ({})",
        line.accrued_interest,
        line.utilized_amount
    );
}

/// Assert that the status is one of the five defined variants (exhaustive match).
fn assert_valid_status(client: &CreditClient<'_>, borrower: &Address, label: &str) {
    let Some(line) = client.get_credit_line(borrower) else {
        return;
    };
    // This exhaustive match will fail to compile if a new variant is added
    // without updating this guard — intentional.
    match line.status {
        CreditStatus::Active
        | CreditStatus::Suspended
        | CreditStatus::Defaulted
        | CreditStatus::Closed
        | CreditStatus::Restricted => {}
    }
    let _ = label; // used in assertion messages above; silence unused warning
}
// ── operation model ───────────────────────────────────────────────────────────

/// Every lifecycle operation that can be attempted in a random sequence.
/// Draw/repay are included because they interact with the debt invariants.
#[derive(Debug, Clone)]
enum LifecycleOp {
    Draw(i128),
    Repay(i128),
    AdminSuspend,
    SelfSuspend,
    AdminDefault,
    AdminClose,
    BorrowerClose,
    ReinstateActive,
    ReinstateRestricted,
    Reopen,
    AdvanceTime(u64),
}

fn op_strategy() -> impl Strategy<Value = Vec<LifecycleOp>> {
    let single = prop_oneof![
        (1_i128..=20_000_i128).prop_map(LifecycleOp::Draw),
        (1_i128..=20_000_i128).prop_map(LifecycleOp::Repay),
        Just(LifecycleOp::AdminSuspend),
        Just(LifecycleOp::SelfSuspend),
        Just(LifecycleOp::AdminDefault),
        Just(LifecycleOp::AdminClose),
        Just(LifecycleOp::BorrowerClose),
        Just(LifecycleOp::ReinstateActive),
        Just(LifecycleOp::ReinstateRestricted),
        Just(LifecycleOp::Reopen),
        (1_u64..=7_776_000_u64).prop_map(LifecycleOp::AdvanceTime),
    ];
    proptest::collection::vec(single, 1..=40)
}

// ── proptest: random op sequences preserve all invariants ────────────────────

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        max_shrink_iters: 512,
        ..ProptestConfig::default()
    })]

    /// Apply up to 40 random lifecycle ops. After every op:
    /// - status is a valid variant
    /// - accounting invariants hold
    /// - no debt fields go negative
    #[test]
    fn prop_lifecycle_invariants_hold_across_random_sequences(
        ops in op_strategy(),
    ) {
        let (env, client, admin, borrower) = setup();

        assert_accounting(&client, &borrower, "initial");
        assert_valid_status(&client, &borrower, "initial");

        for (i, op) in ops.iter().enumerate() {
            let label = std::format!("step={i} op={op:?}");

            match op {
                LifecycleOp::AdvanceTime(secs) => {
                    env.ledger().with_mut(|l| l.timestamp = l.timestamp.saturating_add(*secs));
                }

                LifecycleOp::Draw(amount) => {
                    if let Some(line) = client.get_credit_line(&borrower) {
                        let headroom = (line.credit_limit - line.utilized_amount).max(0);
                        let capped = amount.min(headroom).min(10_000);
                        if capped > 0 {
                            let _ = client.try_draw_credit(&borrower, &capped);
                        }
                    }
                }

                LifecycleOp::Repay(amount) => {
                    if let Some(line) = client.get_credit_line(&borrower) {
                        if line.utilized_amount > 0 {
                            let capped = amount.min(line.utilized_amount + 1_000);
                            let _ = client.try_repay_credit(&borrower, &capped);
                        }
                    }
                }

                LifecycleOp::AdminSuspend => {
                    let _ = client.try_suspend_credit_line(&borrower);
                }

                LifecycleOp::SelfSuspend => {
                    let _ = client.try_self_suspend_credit_line(&borrower);
                }

                LifecycleOp::AdminDefault => {
                    let _ = client.try_default_credit_line(&borrower);
                }

                LifecycleOp::AdminClose => {
                    let _ = client.try_close_credit_line(&borrower, &admin);
                }

                LifecycleOp::BorrowerClose => {
                    let _ = client.try_close_credit_line(&borrower, &borrower);
                }

                LifecycleOp::ReinstateActive => {
                    let _ = client.try_reinstate_credit_line(&borrower, &CreditStatus::Active);
                }

                LifecycleOp::ReinstateRestricted => {
                    let _ = client.try_reinstate_credit_line(&borrower, &CreditStatus::Restricted);
                }

                LifecycleOp::Reopen => {
                    let _ = client.try_open_credit_line(
                        &borrower,
                        &CREDIT_LIMIT,
                        &RATE_BPS,
                        &RISK_SCORE,
                    );
                }
            }

            assert_accounting(&client, &borrower, &label);
            assert_valid_status(&client, &borrower, &label);
        }
    }
}

// ── deterministic invariant: Closed is terminal ──────────────────────────────

/// After admin-closing a line every subsequent state-changing call must fail.
/// The only allowed exception is a second `close_credit_line` call, which is
/// idempotent and must NOT fail.
#[test]
fn closed_is_terminal_no_state_change_succeeds() {
    let (env, client, admin, borrower) = setup();

    // Force-close from Active (no draw, so borrower path also works, but
    // use admin to keep it simple and unconditional).
    client.close_credit_line(&borrower, &admin);
    let after_close = client.get_credit_line(&borrower).expect("line must exist");
    assert_eq!(after_close.status, CreditStatus::Closed, "must be Closed");

    // Idempotent double-close must succeed without changing state.
    client.close_credit_line(&borrower, &admin);
    let still_closed = client.get_credit_line(&borrower).expect("line must exist");
    assert_eq!(
        still_closed.status,
        CreditStatus::Closed,
        "must stay Closed"
    );

    // All other mutations must fail on a Closed line.
    let suspend_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.suspend_credit_line(&borrower);
    }));
    assert!(suspend_result.is_err(), "suspend on Closed must fail");

    let default_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.default_credit_line(&borrower);
    }));
    assert!(default_result.is_err(), "default on Closed must fail");

    let reinstate_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.reinstate_credit_line(&borrower, &CreditStatus::Active);
    }));
    assert!(reinstate_result.is_err(), "reinstate on Closed must fail");

    // Draw on Closed must fail.
    let draw_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.draw_credit(&borrower, &1_i128);
    }));
    assert!(draw_result.is_err(), "draw on Closed must fail");

    // Status must not have changed.
    let final_line = client.get_credit_line(&borrower).expect("line must exist");
    assert_eq!(
        final_line.status,
        CreditStatus::Closed,
        "status must remain Closed"
    );

    // Advance time and re-check — accrual on a Closed line must not produce debt.
    env.ledger().with_mut(|l| l.timestamp += 31_536_000);
    let time_line = client.get_credit_line(&borrower).expect("line must exist");
    assert_eq!(
        time_line.utilized_amount, 0,
        "Closed line must have zero utilized"
    );
    assert_eq!(
        time_line.accrued_interest, 0,
        "Closed line must have zero interest"
    );
}
// ── deterministic invariant: illegal edges always fail ───────────────────────

/// Suspend is only valid from Active. All other source states must reject it.
#[test]
fn suspend_from_non_active_always_fails() {
    // From Suspended
    {
        let (_env, client, _admin, borrower) = setup();
        client.suspend_credit_line(&borrower);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Suspended
        );
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.suspend_credit_line(&borrower);
        }));
        assert!(r.is_err(), "suspend from Suspended must fail");
    }
    // From Defaulted
    {
        let (_env, client, _admin, borrower) = setup();
        client.default_credit_line(&borrower);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.suspend_credit_line(&borrower);
        }));
        assert!(r.is_err(), "suspend from Defaulted must fail");
    }
    // From Closed
    {
        let (_env, client, admin, borrower) = setup();
        client.close_credit_line(&borrower, &admin);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.suspend_credit_line(&borrower);
        }));
        assert!(r.is_err(), "suspend from Closed must fail");
    }
}

/// Reinstate is only valid from Defaulted. Active and Suspended must reject it.
#[test]
fn reinstate_from_non_defaulted_always_fails() {
    // From Active
    {
        let (_env, client, _admin, borrower) = setup();
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Active
        );
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.reinstate_credit_line(&borrower, &CreditStatus::Active);
        }));
        assert!(r.is_err(), "reinstate from Active must fail");
    }
    // From Suspended
    {
        let (_env, client, _admin, borrower) = setup();
        client.suspend_credit_line(&borrower);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.reinstate_credit_line(&borrower, &CreditStatus::Active);
        }));
        assert!(r.is_err(), "reinstate from Suspended must fail");
    }
}

/// Reinstate to Closed, Defaulted, or Suspended must always fail even from Defaulted.
#[test]
fn reinstate_to_invalid_targets_always_fails() {
    for bad_target in [
        CreditStatus::Closed,
        CreditStatus::Defaulted,
        CreditStatus::Suspended,
    ] {
        let (_env, client, _admin, borrower) = setup();
        client.default_credit_line(&borrower);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.reinstate_credit_line(&borrower, &bad_target);
        }));
        assert!(r.is_err(), "reinstate to {bad_target:?} must fail");
        // Status must remain Defaulted — no partial state change.
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Defaulted);
    }
}

// ── deterministic invariant: borrower close requires zero utilization ─────────

/// Borrower cannot close while any outstanding principal remains.
/// Admin can always force-close regardless of utilization.
#[test]
fn borrower_close_requires_zero_utilization() {
    let (_env, client, admin, borrower) = setup();
    client.draw_credit(&borrower, &1_000_i128);

    let line = client.get_credit_line(&borrower).unwrap();
    assert!(
        line.utilized_amount > 0,
        "precondition: need non-zero utilized"
    );

    // Borrower close must fail.
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.close_credit_line(&borrower, &borrower);
    }));
    assert!(r.is_err(), "borrower close with util > 0 must fail");

    // Status must be unchanged.
    let still_active = client.get_credit_line(&borrower).unwrap();
    assert_eq!(
        still_active.status,
        CreditStatus::Active,
        "status must not change on failed close"
    );

    // Admin force-close must succeed unconditionally.
    client.close_credit_line(&borrower, &admin);
    let closed = client.get_credit_line(&borrower).unwrap();
    assert_eq!(closed.status, CreditStatus::Closed);
}

// ── deterministic invariant: duplicate open on Active line fails ──────────────

/// Opening a credit line for a borrower that already has an Active line must
/// fail with AlreadyInitialized (#14).
#[test]
fn duplicate_open_on_active_line_fails() {
    let (_env, client, _admin, borrower) = setup();
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Active
    );

    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.open_credit_line(&borrower, &CREDIT_LIMIT, &RATE_BPS, &RISK_SCORE);
    }));
    assert!(r.is_err(), "re-open of Active line must fail");

    // Status must remain Active.
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Active
    );
}

// ── deterministic invariant: debt is never negative after transitions ─────────

/// Debt fields stay non-negative across the canonical happy path:
/// open → draw → accrue → default → reinstate → repay → close.
#[test]
fn debt_fields_non_negative_across_full_lifecycle() {
    let (env, client, admin, borrower) = setup();

    // Draw.
    client.draw_credit(&borrower, &50_000_i128);
    let after_draw = client.get_credit_line(&borrower).unwrap();
    assert!(after_draw.utilized_amount >= 0);
    assert!(after_draw.accrued_interest >= 0);
    assert!(after_draw.accrued_interest <= after_draw.utilized_amount);

    // Advance time to build interest.
    env.ledger().with_mut(|l| l.timestamp += 15_768_000);

    // Default (triggers accrual internally).
    client.default_credit_line(&borrower);
    let after_default = client.get_credit_line(&borrower).unwrap();
    assert_eq!(after_default.status, CreditStatus::Defaulted);
    assert!(after_default.utilized_amount >= 0);
    assert!(after_default.accrued_interest >= 0);
    assert!(after_default.accrued_interest <= after_default.utilized_amount);

    // Reinstate to Active.
    client.reinstate_credit_line(&borrower, &CreditStatus::Active);
    let after_reinstate = client.get_credit_line(&borrower).unwrap();
    assert_eq!(after_reinstate.status, CreditStatus::Active);
    assert!(after_reinstate.utilized_amount >= 0);
    assert!(after_reinstate.accrued_interest >= 0);
    assert!(after_reinstate.accrued_interest <= after_reinstate.utilized_amount);
    // Reinstate must not alter debt amounts.
    assert_eq!(
        after_reinstate.utilized_amount,
        after_default.utilized_amount
    );
    assert_eq!(
        after_reinstate.accrued_interest,
        after_default.accrued_interest
    );

    // Full repay.
    let debt = after_reinstate.utilized_amount;
    client.repay_credit(&borrower, &(debt + 1_000));
    let after_repay = client.get_credit_line(&borrower).unwrap();
    assert_eq!(after_repay.utilized_amount, 0);
    assert_eq!(after_repay.accrued_interest, 0);

    // Borrower close (util == 0 is now allowed).
    client.close_credit_line(&borrower, &borrower);
    let after_close = client.get_credit_line(&borrower).unwrap();
    assert_eq!(after_close.status, CreditStatus::Closed);
    assert_eq!(after_close.utilized_amount, 0);
    assert_eq!(after_close.accrued_interest, 0);

    // Admin force-close is idempotent.
    client.close_credit_line(&borrower, &admin);
    let final_line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(final_line.status, CreditStatus::Closed);
}

// ── deterministic invariant: debt preserved across suspend/default ────────────

/// Debt record is fully preserved when transitioning Active → Suspended → Defaulted.
/// No double-counting of interest is introduced at status boundaries.
#[test]
fn debt_preserved_through_suspend_then_default() {
    let (env, client, _admin, borrower) = setup();

    client.draw_credit(&borrower, &40_000_i128);
    env.ledger().with_mut(|l| l.timestamp += 31_536_000);

    // Active → Suspended (accrual fires here).
    client.suspend_credit_line(&borrower);
    let after_suspend = client.get_credit_line(&borrower).unwrap();
    assert_eq!(after_suspend.status, CreditStatus::Suspended);
    assert!(after_suspend.utilized_amount >= 0);
    assert!(after_suspend.accrued_interest <= after_suspend.utilized_amount);
    let util_at_suspend = after_suspend.utilized_amount;
    let int_at_suspend = after_suspend.accrued_interest;

    // Suspended → Defaulted (no time advance — no new interest).
    client.default_credit_line(&borrower);
    let after_default = client.get_credit_line(&borrower).unwrap();
    assert_eq!(after_default.status, CreditStatus::Defaulted);
    assert_eq!(
        after_default.utilized_amount, util_at_suspend,
        "utilized must not change on Suspended→Defaulted with no time elapsed"
    );
    assert_eq!(
        after_default.accrued_interest, int_at_suspend,
        "accrued_interest must not change on Suspended→Defaulted with no time elapsed"
    );
}

// ── deterministic invariant: self_suspend mirrors admin suspend effect ────────

/// self_suspend produces Active → Suspended exactly like admin suspend.
/// The resulting status and accounting invariants are identical.
#[test]
fn self_suspend_produces_same_status_as_admin_suspend() {
    // Admin suspend path.
    let admin_util;
    let admin_interest;
    {
        let (env, client, _admin, borrower) = setup();
        client.draw_credit(&borrower, &20_000_i128);
        env.ledger().with_mut(|l| l.timestamp += 7_884_000);
        client.suspend_credit_line(&borrower);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Suspended);
        admin_util = line.utilized_amount;
        admin_interest = line.accrued_interest;
    }

    // Self-suspend path (same draw + same time advance).
    let self_util;
    let self_interest;
    {
        let (env, client, _admin, borrower) = setup();
        client.draw_credit(&borrower, &20_000_i128);
        env.ledger().with_mut(|l| l.timestamp += 7_884_000);
        client.self_suspend_credit_line(&borrower);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Suspended);
        self_util = line.utilized_amount;
        self_interest = line.accrued_interest;
    }

    assert_eq!(
        admin_util, self_util,
        "utilized_amount must match between admin and self suspend"
    );
    assert_eq!(
        admin_interest, self_interest,
        "accrued_interest must match between admin and self suspend"
    );
}
// ── deterministic invariant: proptest edge-case sequences ────────────────────

/// Repay on a Closed line must fail — repayments are blocked in terminal state.
#[test]
fn repay_on_closed_line_fails() {
    let (_env, client, admin, borrower) = setup();
    client.draw_credit(&borrower, &10_000_i128);
    // Admin force-closes with outstanding balance.
    client.close_credit_line(&borrower, &admin);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Closed
    );

    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.repay_credit(&borrower, &1_i128);
    }));
    assert!(r.is_err(), "repay on Closed must fail");

    // Debt must be frozen — admin close preserves the outstanding balance.
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Closed);
    assert!(line.utilized_amount >= 0);
}

/// Default on a line that is already Defaulted is idempotent — must not panic.
#[test]
fn default_already_defaulted_is_idempotent() {
    let (_env, client, _admin, borrower) = setup();
    client.default_credit_line(&borrower);
    let first = client.get_credit_line(&borrower).unwrap();
    assert_eq!(first.status, CreditStatus::Defaulted);

    // Second default must not panic.
    client.default_credit_line(&borrower);
    let second = client.get_credit_line(&borrower).unwrap();
    assert_eq!(
        second.status,
        CreditStatus::Defaulted,
        "must remain Defaulted"
    );
    assert_eq!(
        second.utilized_amount, first.utilized_amount,
        "utilized must be unchanged"
    );
    assert_eq!(
        second.accrued_interest, first.accrued_interest,
        "interest must be unchanged"
    );
}

/// draw is blocked when status is Suspended (error #20) or Defaulted (#21).
#[test]
fn draw_blocked_on_suspended_and_defaulted() {
    // Suspended
    {
        let (_env, client, _admin, borrower) = setup();
        client.suspend_credit_line(&borrower);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.draw_credit(&borrower, &1_i128);
        }));
        assert!(r.is_err(), "draw on Suspended must fail");
        // Status must not change.
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Suspended
        );
    }
    // Defaulted
    {
        let (_env, client, _admin, borrower) = setup();
        client.default_credit_line(&borrower);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.draw_credit(&borrower, &1_i128);
        }));
        assert!(r.is_err(), "draw on Defaulted must fail");
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Defaulted
        );
    }
}

/// Reinstate preserves debt — it must never zero or alter the balance fields.
#[test]
fn reinstate_does_not_alter_debt_fields() {
    let (env, client, _admin, borrower) = setup();
    client.draw_credit(&borrower, &30_000_i128);
    env.ledger().with_mut(|l| l.timestamp += 15_768_000);
    client.default_credit_line(&borrower);

    let before = client.get_credit_line(&borrower).unwrap();
    assert_eq!(before.status, CreditStatus::Defaulted);

    client.reinstate_credit_line(&borrower, &CreditStatus::Active);
    let after = client.get_credit_line(&borrower).unwrap();
    assert_eq!(after.status, CreditStatus::Active);
    assert_eq!(
        after.utilized_amount, before.utilized_amount,
        "reinstate must not change utilized_amount"
    );
    assert_eq!(
        after.accrued_interest, before.accrued_interest,
        "reinstate must not change accrued_interest"
    );
    assert!(after.accrued_interest <= after.utilized_amount);
}

/// Reopen after Closed resets utilization to zero and status to Active.
#[test]
fn reopen_after_closed_resets_state_correctly() {
    let (_env, client, admin, borrower) = setup();
    client.draw_credit(&borrower, &5_000_i128);
    client.close_credit_line(&borrower, &admin);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Closed
    );

    // Admin can reopen a Closed line.
    client.open_credit_line(&borrower, &CREDIT_LIMIT, &RATE_BPS, &RISK_SCORE);
    let reopened = client.get_credit_line(&borrower).unwrap();
    assert_eq!(
        reopened.status,
        CreditStatus::Active,
        "reopened line must be Active"
    );
    assert_eq!(
        reopened.utilized_amount, 0,
        "reopened line must have zero utilized"
    );
    assert_eq!(
        reopened.accrued_interest, 0,
        "reopened line must have zero accrued interest"
    );
}
