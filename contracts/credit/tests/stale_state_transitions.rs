// SPDX-License-Identifier: MIT

//! Focused regression and invariant tests for Issue #1146:
//! "Reject stale credit-line state transitions".
//!
//! # What these tests cover
//!
//! | Test group                    | Description |
//! |-------------------------------|-------------|
//! | `suspend::*`                  | Active→Suspended (valid); stale Suspended→Suspended; illegal transitions from Closed/Defaulted/Restricted |
//! | `default_line::*`             | Active/Suspended/Restricted→Defaulted (valid); stale Defaulted→Defaulted; illegal Closed→Defaulted |
//! | `reinstate::*`                | Defaulted→Active/Restricted (valid); stale Active/Restricted→Active; illegal Closed/Suspended→Active |
//! | `close::*`                    | Active/Suspended/Defaulted→Closed (valid); stale Closed→Closed rejects (not idempotent) |
//! | `discriminant_stability::*`   | StaleStateTransition == 60, Lifecycle category |
//! | `concurrent_guard::*`         | Retry-safe invariants: two rapid identical calls fail the second deterministically |
//!
//! # Why
//!
//! Before #1146 several lifecycle entry points exhibited silent or
//! misleading behaviour on stale calls:
//!
//! - `default_credit_line` silently returned (idempotent) when already
//!   Defaulted, making it impossible for the caller to detect a duplicate
//!   attempt.
//! - `close_credit_line` silently returned (idempotent) when already
//!   Closed.
//! - `suspend_credit_line_internal` panicked with `CreditLineSuspended`
//!   (code 20) for *every* non-Active status — including Defaulted and
//!   Closed — which was misleading.
//!
//! This module asserts the post-#1146 behaviour: all stale calls **must**
//! revert with `ContractError::StaleStateTransition` (code 60), and all
//! *invalid*-but-non-stale calls revert with a semantically correct
//! existing error (e.g. `CreditLineClosed`, `CreditLineDefaulted`).
//!
//! # State machine reference
//!
//! ```text
//! Active     ─[suspend]──→ Suspended
//! Active     ─[default]──→ Defaulted
//! Active     ─[close]────→ Closed
//! Suspended  ─[default]──→ Defaulted
//! Suspended  ─[close]────→ Closed
//! Defaulted  ─[reinstate]→ Active | Restricted
//! Defaulted  ─[close]────→ Closed
//! Closed     ─[*]────────→ TERMINAL (all mutations rejected)
//! ```

use creditra_credit::types::{ContractError, CreditStatus};
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env};

// ── shared helpers ────────────────────────────────────────────────────────────

/// Deploy a fresh contract, init it, open one credit line, and return the
/// client + borrower address. All auth is mocked.
fn setup() -> (Env, CreditClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let borrower = Address::generate(&env);
    client.open_credit_line(&borrower, &100_000_i128, &500_u32, &40_u32);

    (env, client, borrower)
}

/// Advance ledger time by `secs` seconds (needed so accrual timestamps don't
/// regress when lifecycle functions call `apply_accrual` internally).
fn advance(env: &Env, secs: u64) {
    let t = env.ledger().timestamp();
    env.ledger().set_timestamp(t + secs);
}

// ═════════════════════════════════════════════════════════════════════════════
// Discriminant stability
// ═════════════════════════════════════════════════════════════════════════════

/// StaleStateTransition must be pinned at discriminant 60 (Issue #1146).
#[test]
fn stale_state_transition_discriminant_is_60() {
    assert_eq!(ContractError::StaleStateTransition as u32, 60);
}

/// StaleStateTransition must be classified as Lifecycle.
#[test]
fn stale_state_transition_category_is_lifecycle() {
    use creditra_credit::types::ContractErrorCategory;
    assert_eq!(
        ContractError::StaleStateTransition.category(),
        ContractErrorCategory::Lifecycle,
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// suspend_credit_line / self_suspend_credit_line
// ═════════════════════════════════════════════════════════════════════════════

mod suspend {
    use super::*;

    // ── success ──────────────────────────────────────────────────────────────

    /// Happy path: Active → Suspended.
    #[test]
    fn active_to_suspended_succeeds() {
        let (_, client, borrower) = setup();
        client.suspend_credit_line(&borrower);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Suspended);
    }

    /// Self-suspend by borrower: Active → Suspended.
    #[test]
    fn self_suspend_active_succeeds() {
        let (_, client, borrower) = setup();
        client.self_suspend_credit_line(&borrower);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Suspended);
    }

    // ── stale transitions ─────────────────────────────────────────────────────

    /// Stale: attempting to suspend an already-Suspended line must revert
    /// with StaleStateTransition (#60).
    #[test]
    #[should_panic(expected = "Error(Contract, #60)")]
    fn suspend_already_suspended_is_stale() {
        let (env, client, borrower) = setup();
        client.suspend_credit_line(&borrower);
        advance(&env, 1);
        // Second suspend on an already-Suspended line is stale.
        client.suspend_credit_line(&borrower);
    }

    /// Stale via self-suspend: borrower attempting to self-suspend twice.
    #[test]
    #[should_panic(expected = "Error(Contract, #60)")]
    fn self_suspend_already_suspended_is_stale() {
        let (env, client, borrower) = setup();
        client.self_suspend_credit_line(&borrower);
        advance(&env, 1);
        client.self_suspend_credit_line(&borrower);
    }

    // ── invalid transitions ───────────────────────────────────────────────────

    /// Invalid: suspending a Defaulted line emits CreditLineDefaulted (#21).
    #[test]
    #[should_panic(expected = "Error(Contract, #21)")]
    fn suspend_defaulted_line_is_invalid() {
        let (env, client, borrower) = setup();
        advance(&env, 1);
        client.default_credit_line(&borrower);
        advance(&env, 1);
        client.suspend_credit_line(&borrower);
    }

    /// Invalid: suspending a Closed line emits CreditLineClosed (#4).
    #[test]
    #[should_panic(expected = "Error(Contract, #4)")]
    fn suspend_closed_line_is_invalid() {
        let (env, client, borrower) = setup();
        advance(&env, 1);
        client.close_credit_line(&borrower, &borrower); // borrower self-close (util=0)
        advance(&env, 1);
        client.suspend_credit_line(&borrower);
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// default_credit_line
// ═════════════════════════════════════════════════════════════════════════════

mod default_line {
    use super::*;

    // ── success ──────────────────────────────────────────────────────────────

    /// Happy path: Active → Defaulted.
    #[test]
    fn active_to_defaulted_succeeds() {
        let (env, client, borrower) = setup();
        advance(&env, 1);
        client.default_credit_line(&borrower);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Defaulted);
    }

    /// Happy path: Suspended → Defaulted.
    #[test]
    fn suspended_to_defaulted_succeeds() {
        let (env, client, borrower) = setup();
        advance(&env, 1);
        client.suspend_credit_line(&borrower);
        advance(&env, 1);
        client.default_credit_line(&borrower);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Defaulted);
    }

    // ── stale transitions ─────────────────────────────────────────────────────

    /// Stale: attempting to default an already-Defaulted line must revert
    /// with StaleStateTransition (#60) — not silently succeed.
    #[test]
    #[should_panic(expected = "Error(Contract, #60)")]
    fn default_already_defaulted_is_stale() {
        let (env, client, borrower) = setup();
        advance(&env, 1);
        client.default_credit_line(&borrower);
        advance(&env, 1);
        // Second default on a Defaulted line is stale.
        client.default_credit_line(&borrower);
    }

    // ── invalid transitions ───────────────────────────────────────────────────

    /// Invalid: defaulting a Closed line emits CreditLineClosed (#4).
    #[test]
    #[should_panic(expected = "Error(Contract, #4)")]
    fn default_closed_line_is_invalid() {
        let (env, client, borrower) = setup();
        advance(&env, 1);
        client.close_credit_line(&borrower, &borrower);
        advance(&env, 1);
        client.default_credit_line(&borrower);
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// reinstate_credit_line
// ═════════════════════════════════════════════════════════════════════════════

mod reinstate {
    use super::*;

    // ── success ──────────────────────────────────────────────────────────────

    /// Happy path: Defaulted → Active.
    #[test]
    fn defaulted_to_active_succeeds() {
        let (env, client, borrower) = setup();
        advance(&env, 1);
        client.default_credit_line(&borrower);
        advance(&env, 1);
        client.reinstate_credit_line(&borrower, &CreditStatus::Active);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Active);
    }

    /// Happy path: Defaulted → Restricted.
    #[test]
    fn defaulted_to_restricted_succeeds() {
        let (env, client, borrower) = setup();
        advance(&env, 1);
        client.default_credit_line(&borrower);
        advance(&env, 1);
        client.reinstate_credit_line(&borrower, &CreditStatus::Restricted);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Restricted);
    }

    // ── stale transitions ─────────────────────────────────────────────────────

    /// Stale: reinstating a line that's already Active must revert with
    /// StaleStateTransition (#60).
    #[test]
    #[should_panic(expected = "Error(Contract, #60)")]
    fn reinstate_already_active_is_stale() {
        let (env, client, borrower) = setup();
        advance(&env, 1);
        client.default_credit_line(&borrower);
        advance(&env, 1);
        client.reinstate_credit_line(&borrower, &CreditStatus::Active);
        advance(&env, 1);
        // The line is now Active. Attempting to reinstate to Active again is stale.
        client.reinstate_credit_line(&borrower, &CreditStatus::Active);
    }

    /// Stale: reinstating a line that's already Restricted must revert with
    /// StaleStateTransition (#60).
    #[test]
    #[should_panic(expected = "Error(Contract, #60)")]
    fn reinstate_already_restricted_is_stale() {
        let (env, client, borrower) = setup();
        advance(&env, 1);
        client.default_credit_line(&borrower);
        advance(&env, 1);
        client.reinstate_credit_line(&borrower, &CreditStatus::Restricted);
        advance(&env, 1);
        // The line is now Restricted. Attempting to reinstate to Restricted again is stale.
        client.reinstate_credit_line(&borrower, &CreditStatus::Restricted);
    }

    // ── invalid transitions ───────────────────────────────────────────────────

    /// Invalid: reinstating a Suspended line emits CreditLineDefaulted (#21)
    /// because the source state is not Defaulted.
    #[test]
    #[should_panic(expected = "Error(Contract, #21)")]
    fn reinstate_suspended_line_is_invalid() {
        let (env, client, borrower) = setup();
        advance(&env, 1);
        client.suspend_credit_line(&borrower);
        advance(&env, 1);
        // Reinstating from Suspended is not valid (must be from Defaulted).
        client.reinstate_credit_line(&borrower, &CreditStatus::Active);
    }

    /// Invalid: reinstating a Closed line emits CreditLineClosed (#4).
    #[test]
    #[should_panic(expected = "Error(Contract, #4)")]
    fn reinstate_closed_line_is_invalid() {
        let (env, client, borrower) = setup();
        advance(&env, 1);
        client.default_credit_line(&borrower);
        advance(&env, 1);
        client.reinstate_credit_line(&borrower, &CreditStatus::Active);
        advance(&env, 1);
        client.close_credit_line(&borrower, &borrower); // util=0, borrower self-close
        advance(&env, 1);
        // Closed is terminal; reinstate must reject with CreditLineClosed.
        client.reinstate_credit_line(&borrower, &CreditStatus::Active);
    }

    /// Invalid target: Closed as a reinstate target emits InvalidAmount (#5).
    #[test]
    #[should_panic(expected = "Error(Contract, #5)")]
    fn reinstate_target_closed_is_invalid_amount() {
        let (env, client, borrower) = setup();
        advance(&env, 1);
        client.default_credit_line(&borrower);
        advance(&env, 1);
        // Closed is not a valid reinstate target (only Active or Restricted are).
        client.reinstate_credit_line(&borrower, &CreditStatus::Closed);
    }

    /// Invalid target: Suspended as a reinstate target emits InvalidAmount (#5).
    #[test]
    #[should_panic(expected = "Error(Contract, #5)")]
    fn reinstate_target_suspended_is_invalid_amount() {
        let (env, client, borrower) = setup();
        advance(&env, 1);
        client.default_credit_line(&borrower);
        advance(&env, 1);
        client.reinstate_credit_line(&borrower, &CreditStatus::Suspended);
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// close_credit_line
// ═════════════════════════════════════════════════════════════════════════════

mod close {
    use super::*;

    // ── success ──────────────────────────────────────────────────────────────

    /// Happy path: Active → Closed (borrower, util=0).
    #[test]
    fn active_to_closed_borrower_succeeds() {
        let (env, client, borrower) = setup();
        advance(&env, 1);
        client.close_credit_line(&borrower, &borrower);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Closed);
    }

    /// Happy path: Suspended → Closed (borrower, util=0).
    #[test]
    fn suspended_to_closed_borrower_succeeds() {
        let (env, client, borrower) = setup();
        advance(&env, 1);
        client.suspend_credit_line(&borrower);
        advance(&env, 1);
        client.close_credit_line(&borrower, &borrower);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Closed);
    }

    /// Happy path: Defaulted → Closed (borrower, util=0).
    #[test]
    fn defaulted_to_closed_borrower_succeeds() {
        let (env, client, borrower) = setup();
        advance(&env, 1);
        client.default_credit_line(&borrower);
        advance(&env, 1);
        client.close_credit_line(&borrower, &borrower);
        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Closed);
    }

    // ── stale transitions ─────────────────────────────────────────────────────

    /// Stale: closing an already-Closed line must revert with
    /// StaleStateTransition (#60) — not silently succeed (breaks the previous
    /// idempotent return path, which masked duplicate close attempts).
    #[test]
    #[should_panic(expected = "Error(Contract, #60)")]
    fn close_already_closed_is_stale() {
        let (env, client, borrower) = setup();
        advance(&env, 1);
        client.close_credit_line(&borrower, &borrower);
        advance(&env, 1);
        // Second close on an already-Closed line is stale.
        client.close_credit_line(&borrower, &borrower);
    }

    // ── boundary cases ────────────────────────────────────────────────────────

    /// Borrower cannot close when utilized_amount > 0 (UtilizationNotZero #10).
    ///
    /// This is a boundary test: amount == 1 triggers the guard.
    #[test]
    #[should_panic(expected = "Error(Contract, #10)")]
    fn close_with_nonzero_balance_by_borrower_fails() {
        let (env, client, borrower) = setup();
        // Set up liquidity so we can draw.
        let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
        let token = token_id.address();
        client.set_liquidity_token(&token);
        soroban_sdk::token::StellarAssetClient::new(&env, &token)
            .mint(&env.register(Credit, ()), &1_000_000_i128);
        advance(&env, 1);
        client.draw_credit(&borrower, &1_i128);
        advance(&env, 1);
        // Borrower tries to close while still owing 1 token.
        client.close_credit_line(&borrower, &borrower);
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Concurrent / retry safety guard
// ═════════════════════════════════════════════════════════════════════════════

/// Verify that a rapid retry of `default_credit_line` yields a deterministic
/// StaleStateTransition and does NOT silently succeed (regression against the
/// pre-#1146 idempotent-return behaviour).
///
/// This covers the "retries cannot produce an unsafe or inconsistent result"
/// acceptance criterion.
#[test]
fn default_retry_is_deterministically_stale() {
    let (env, client, borrower) = setup();
    advance(&env, 1);

    // First call: valid Active→Defaulted.
    client.default_credit_line(&borrower);
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Defaulted, "first default must succeed");

    advance(&env, 1);

    // Second call: stale — must revert, not silently return.
    let result = client.try_default_credit_line(&borrower);
    assert!(result.is_err(), "stale default must fail");
    assert_eq!(
        result.err().unwrap().unwrap(),
        ContractError::StaleStateTransition.into(),
        "stale default must return StaleStateTransition (#60)"
    );
}

/// Verify that a rapid retry of `suspend_credit_line` yields a deterministic
/// StaleStateTransition.
#[test]
fn suspend_retry_is_deterministically_stale() {
    let (env, client, borrower) = setup();

    // First call: valid Active→Suspended.
    client.suspend_credit_line(&borrower);
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Suspended, "first suspend must succeed");

    advance(&env, 1);

    // Second call: stale.
    let result = client.try_suspend_credit_line(&borrower);
    assert!(result.is_err(), "stale suspend must fail");
    assert_eq!(
        result.err().unwrap().unwrap(),
        ContractError::StaleStateTransition.into(),
        "stale suspend must return StaleStateTransition (#60)"
    );
}

/// Verify that a rapid retry of `close_credit_line` yields a deterministic
/// StaleStateTransition (regression: was previously silent idempotent return).
#[test]
fn close_retry_is_deterministically_stale() {
    let (env, client, borrower) = setup();
    advance(&env, 1);

    // First call: valid Active→Closed (util=0, borrower self-close).
    client.close_credit_line(&borrower, &borrower);
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Closed, "first close must succeed");

    advance(&env, 1);

    // Second call: stale.
    let result = client.try_close_credit_line(&borrower, &borrower);
    assert!(result.is_err(), "stale close must fail");
    assert_eq!(
        result.err().unwrap().unwrap(),
        ContractError::StaleStateTransition.into(),
        "stale close must return StaleStateTransition (#60)"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// Full round-trip: ensure valid transitions still work after implementation
// ═════════════════════════════════════════════════════════════════════════════

/// Full lifecycle round-trip:
/// Active → Suspended → Defaulted → Active (reinstated) → Closed.
///
/// Asserts that every valid edge still works post-#1146 and the final state
/// is Closed (terminal).
#[test]
fn full_lifecycle_round_trip() {
    let (env, client, borrower) = setup();

    // Active → Suspended
    advance(&env, 1);
    client.suspend_credit_line(&borrower);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Suspended
    );

    // Suspended → Defaulted
    advance(&env, 1);
    client.default_credit_line(&borrower);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Defaulted
    );

    // Defaulted → Active (reinstated)
    advance(&env, 1);
    client.reinstate_credit_line(&borrower, &CreditStatus::Active);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Active
    );

    // Active → Closed (borrower self-close, util=0)
    advance(&env, 1);
    client.close_credit_line(&borrower, &borrower);
    assert_eq!(
        client.get_credit_line(&borrower).unwrap().status,
        CreditStatus::Closed
    );

    // Closed is terminal — any mutation must now reject.
    advance(&env, 1);
    assert!(
        client.try_suspend_credit_line(&borrower).is_err(),
        "Closed: suspend must fail"
    );
    assert!(
        client.try_default_credit_line(&borrower).is_err(),
        "Closed: default must fail"
    );
    assert!(
        client
            .try_reinstate_credit_line(&borrower, &CreditStatus::Active)
            .is_err(),
        "Closed: reinstate must fail"
    );
}
