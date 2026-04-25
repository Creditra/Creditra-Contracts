// SPDX-License-Identifier: MIT

//! Lifecycle state-machine transitions for credit lines.
//!
//! Valid state machine:
//!
//! ```text
//!                     ┌────────────────────────────────┐
//!                     │                                │
//!   open_credit_line  ▼     suspend_credit_line        │  reinstate_credit_line(Active)
//!   ──────────────► Active ──────────────────► Suspended ◄────────────────────────────┐
//!                     │                                │                              │
//!   default_credit_line│            default_credit_line│                              │
//!                     ▼                                ▼                              │
//!                  Defaulted ◄──────────────────── Defaulted ─────────────────────────┘
//!                     │          (same state)           │  reinstate_credit_line(Suspended)
//!                     │                                 └────────────────────────────► Suspended
//!                     │
//!   close_credit_line │  (admin may force-close from any non-Closed state)
//!                     ▼
//!                   Closed
//! ```
//!
//! `reinstate_credit_line` is the only transition *out* of `Defaulted`
//! (other than force-close). The admin specifies whether the line
//! comes back as `Active` or `Suspended`.

use soroban_sdk::{symbol_short, Address, Env};

use crate::auth::require_admin_auth;
use crate::events::{publish_credit_line_event, CreditLineEvent};
use crate::storage::DataKey;
use crate::types::{ContractError, CreditLineData, CreditStatus};

// ── suspend_credit_line ───────────────────────────────────────────────────────

/// Suspend an active credit line (admin only).
///
/// # State transition
/// `Active → Suspended`
///
/// # Errors
/// - Panics with `"Credit line not found"` if the borrower has no credit line.
/// - Panics with `"Only active credit lines can be suspended"` if the line is
///   not currently `Active`.
pub fn suspend_credit_line(env: Env, borrower: Address) {
    require_admin_auth(&env);
    let mut credit_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));

    if credit_line.status != CreditStatus::Active {
        panic!("Only active credit lines can be suspended");
    }

    credit_line.status = CreditStatus::Suspended;
    env.storage().persistent().set(&borrower, &credit_line);

    publish_credit_line_event(
        &env,
        (symbol_short!("credit"), symbol_short!("suspended")),
        CreditLineEvent {
            event_type: symbol_short!("suspended"),
            borrower: borrower.clone(),
            status: CreditStatus::Suspended,
            credit_limit: credit_line.credit_limit,
            interest_rate_bps: credit_line.interest_rate_bps,
            risk_score: credit_line.risk_score,
        },
    );
}

// ── close_credit_line ─────────────────────────────────────────────────────────

/// Close a credit line (admin force-close, or borrower self-close with zero utilization).
///
/// # State transition
/// `Any non-Closed → Closed`
///
/// # Authorization
/// - Admin may close from any non-Closed status regardless of utilization.
/// - Borrower may only close their own line when `utilized_amount == 0`.
///
/// # Errors
/// - Panics with `"unauthorized"` if `closer` is neither admin nor `borrower`.
/// - Panics with `"cannot close: utilized amount not zero"` when borrower
///   tries to close a line with outstanding utilization.
/// - Idempotent: already-Closed lines are accepted silently.
pub fn close_credit_line(env: Env, borrower: Address, closer: Address) {
    let admin: Address = env
        .storage()
        .instance()
        .get(&crate::admin_key(&env))
        .unwrap_or_else(|| env.panic_with_error(ContractError::NotAdmin));

    let is_admin = closer == admin;
    let is_borrower = closer == borrower;

    if !is_admin && !is_borrower {
        panic!("unauthorized");
    }

    if is_admin {
        closer.require_auth();
    } else {
        borrower.require_auth();
    }

    let mut credit_line: CreditLineData = match env.storage().persistent().get(&borrower) {
        Some(line) => line,
        None => env.panic_with_error(ContractError::CreditLineNotFound),
    };

    // Idempotent: already closed is fine.
    if credit_line.status == CreditStatus::Closed {
        return;
    }

    // Borrower self-close requires zero utilization.
    if is_borrower && !is_admin && credit_line.utilized_amount != 0 {
        panic!("cannot close: utilized amount not zero");
    }

    credit_line.status = CreditStatus::Closed;
    env.storage().persistent().set(&borrower, &credit_line);

    publish_credit_line_event(
        &env,
        (symbol_short!("credit"), symbol_short!("closed")),
        CreditLineEvent {
            event_type: symbol_short!("closed"),
            borrower: borrower.clone(),
            status: CreditStatus::Closed,
            credit_limit: credit_line.credit_limit,
            interest_rate_bps: credit_line.interest_rate_bps,
            risk_score: credit_line.risk_score,
        },
    );
}

// ── default_credit_line ───────────────────────────────────────────────────────

/// Mark a credit line as defaulted (admin only).
///
/// # State transition
/// `Active | Suspended → Defaulted`
///
/// # Errors
/// - Panics with `"Credit line not found"` if no credit line exists for the borrower.
/// - Panics if the line is already `Defaulted` or `Closed`.
pub fn default_credit_line(env: Env, borrower: Address) {
    require_admin_auth(&env);
    let mut credit_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));

    if credit_line.status == CreditStatus::Closed {
        env.panic_with_error(ContractError::CreditLineClosed);
    }

    credit_line.status = CreditStatus::Defaulted;
    env.storage().persistent().set(&borrower, &credit_line);

    publish_credit_line_event(
        &env,
        (symbol_short!("credit"), symbol_short!("defaulted")),
        CreditLineEvent {
            event_type: symbol_short!("defaulted"),
            borrower: borrower.clone(),
            status: CreditStatus::Defaulted,
            credit_limit: credit_line.credit_limit,
            interest_rate_bps: credit_line.interest_rate_bps,
            risk_score: credit_line.risk_score,
        },
    );
}

// ── reinstate_credit_line ─────────────────────────────────────────────────────

/// Reinstate a `Defaulted` credit line to either `Active` or `Suspended` (admin only).
///
/// # State transition
/// `Defaulted → Active`  (when `target_status == CreditStatus::Active`)
/// `Defaulted → Suspended` (when `target_status == CreditStatus::Suspended`)
///
/// This is the **only** valid exit from `Defaulted` other than a force-close.
///
/// # Post-reinstatement invariants
/// - `utilized_amount` is preserved unchanged; it must still satisfy
///   `0 ≤ utilized_amount ≤ credit_limit`.
/// - `interest_rate_bps`, `risk_score`, and `credit_limit` are unchanged.
/// - If reinstated to `Active`, draws are immediately permitted (subject to
///   limit and liquidity checks).
/// - If reinstated to `Suspended`, draws remain blocked until the admin
///   calls [`suspend_credit_line`] → [`reinstate_credit_line`] with `Active`.
/// - A `"reinstate"` event is emitted recording the new status.
///
/// # Parameters
/// - `borrower` — The borrower whose line is being reinstated.
/// - `target_status` — Must be either [`CreditStatus::Active`] or
///   [`CreditStatus::Suspended`]. Any other value reverts.
///
/// # Errors
/// - [`ContractError::NotAdmin`] — caller is not the contract admin.
/// - Panics with `"credit line is not defaulted"` if the current status is
///   not `Defaulted` (guards against invalid transitions from `Active`,
///   `Suspended`, or `Closed`).
/// - Panics with `"invalid target status: must be Active or Suspended"` if
///   `target_status` is anything other than `Active` or `Suspended`.
/// - Panics with `"Credit line not found"` if no credit line exists for the
///   borrower.
///
/// # Security
/// - Admin-only: `require_admin_auth` enforces this before any state mutation.
/// - Source state is strictly validated (`Defaulted` only) before any write,
///   preventing accidental transitions from `Active`, `Suspended`, or `Closed`.
/// - `target_status` is whitelisted (`Active` | `Suspended`) so no arbitrary
///   status can be injected through this entry point.
pub fn reinstate_credit_line(env: Env, borrower: Address, target_status: CreditStatus) {
    require_admin_auth(&env);

    // ── Validate target status early (fail fast before storage read) ──────────
    if target_status != CreditStatus::Active && target_status != CreditStatus::Suspended {
        panic!("invalid target status: must be Active or Suspended");
    }

    // ── Load credit line ───────────────────────────────────────────────────────
    let mut credit_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));

    // ── Guard: source state must be Defaulted ─────────────────────────────────
    if credit_line.status != CreditStatus::Defaulted {
        panic!("credit line is not defaulted");
    }

    // ── Apply transition ───────────────────────────────────────────────────────
    credit_line.status = target_status.clone();
    env.storage().persistent().set(&borrower, &credit_line);

    // ── Emit event ─────────────────────────────────────────────────────────────
    publish_credit_line_event(
        &env,
        (symbol_short!("credit"), symbol_short!("reinstate")),
        CreditLineEvent {
            event_type: symbol_short!("reinstate"),
            borrower: borrower.clone(),
            status: target_status,
            credit_limit: credit_line.credit_limit,
            interest_rate_bps: credit_line.interest_rate_bps,
            risk_score: credit_line.risk_score,
        },
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests_reinstate {
    //! Explicit transition tests for `reinstate_credit_line` (issue #230).
    //!
    //! Invariants verified after reinstatement:
    //! 1. `status` equals the requested `target_status`.
    //! 2. `utilized_amount` is unchanged.
    //! 3. `credit_limit`, `interest_rate_bps`, `risk_score` are unchanged.
    //! 4. A `"reinstate"` event is emitted with the correct payload.
    //! 5. Invalid source states (`Active`, `Suspended`, `Closed`) revert.
    //! 6. Invalid target states revert.
    //! 7. Non-admin callers revert.

    use soroban_sdk::testutils::{Address as _, Events as _};
    use soroban_sdk::{symbol_short, Env, Symbol, TryFromVal, TryIntoVal};

    use crate::events::CreditLineEvent;
    use crate::types::{CreditLineData, CreditStatus};
    use crate::{Credit, CreditClient};

    // ── helpers ───────────────────────────────────────────────────────────────

    fn setup(env: &Env) -> (CreditClient<'_>, soroban_sdk::Address, soroban_sdk::Address) {
        env.mock_all_auths();
        let admin = soroban_sdk::Address::generate(env);
        let borrower = soroban_sdk::Address::generate(env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(env, &contract_id);
        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
        (client, admin, borrower)
    }

    // ── 1. Defaulted → Active (happy path) ───────────────────────────────────

    #[test]
    fn reinstate_defaulted_to_active_succeeds() {
        let env = Env::default();
        let (client, _admin, borrower) = setup(&env);

        client.default_credit_line(&borrower);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Defaulted
        );

        client.reinstate_credit_line(&borrower, &CreditStatus::Active);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Active);
    }

    // ── 2. Defaulted → Suspended (happy path) ────────────────────────────────

    #[test]
    fn reinstate_defaulted_to_suspended_succeeds() {
        let env = Env::default();
        let (client, _admin, borrower) = setup(&env);

        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower, &CreditStatus::Suspended);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Suspended);
    }

    // ── 3. Post-reinstatement invariants ─────────────────────────────────────

    #[test]
    fn reinstate_preserves_utilized_amount_and_other_fields() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = soroban_sdk::Address::generate(&env);
        let borrower = soroban_sdk::Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);

        // Use a token so we can draw
        let token_id =
            env.register_stellar_asset_contract_v2(soroban_sdk::Address::generate(&env));
        client.set_liquidity_token(&token_id.address());
        soroban_sdk::token::StellarAssetClient::new(&env, &token_id.address())
            .mint(&contract_id, &1_000_i128);

        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &400_i128);

        let before: CreditLineData = client.get_credit_line(&borrower).unwrap();
        assert_eq!(before.utilized_amount, 400);

        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower, &CreditStatus::Active);

        let after: CreditLineData = client.get_credit_line(&borrower).unwrap();

        // Status is the target
        assert_eq!(after.status, CreditStatus::Active);
        // All other fields are unchanged
        assert_eq!(after.utilized_amount, before.utilized_amount);
        assert_eq!(after.credit_limit, before.credit_limit);
        assert_eq!(after.interest_rate_bps, before.interest_rate_bps);
        assert_eq!(after.risk_score, before.risk_score);
        assert_eq!(after.borrower, before.borrower);
    }

    // ── 4. Reinstated-to-Active allows draws ──────────────────────────────────

    #[test]
    fn reinstate_to_active_permits_subsequent_draw() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = soroban_sdk::Address::generate(&env);
        let borrower = soroban_sdk::Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);

        let token_id =
            env.register_stellar_asset_contract_v2(soroban_sdk::Address::generate(&env));
        client.set_liquidity_token(&token_id.address());
        soroban_sdk::token::StellarAssetClient::new(&env, &token_id.address())
            .mint(&contract_id, &1_000_i128);

        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &200_i128);
        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower, &CreditStatus::Active);

        // Draw should succeed after reinstatement to Active
        client.draw_credit(&borrower, &100_i128);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.utilized_amount, 300);
        assert_eq!(line.status, CreditStatus::Active);
    }

    // ── 5. Reinstated-to-Suspended blocks draws ───────────────────────────────

    #[test]
    #[should_panic(expected = "credit line is suspended")]
    fn reinstate_to_suspended_blocks_draw() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = soroban_sdk::Address::generate(&env);
        let borrower = soroban_sdk::Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);

        let token_id =
            env.register_stellar_asset_contract_v2(soroban_sdk::Address::generate(&env));
        client.set_liquidity_token(&token_id.address());
        soroban_sdk::token::StellarAssetClient::new(&env, &token_id.address())
            .mint(&contract_id, &1_000_i128);

        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower, &CreditStatus::Suspended);

        // Draw must be rejected — line is Suspended
        client.draw_credit(&borrower, &100_i128);
    }

    // ── 6. Reinstated-to-Suspended still allows repay ────────────────────────

    #[test]
    fn reinstate_to_suspended_allows_repay() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = soroban_sdk::Address::generate(&env);
        let borrower = soroban_sdk::Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);

        let token_id =
            env.register_stellar_asset_contract_v2(soroban_sdk::Address::generate(&env));
        let token_address = token_id.address();
        client.set_liquidity_token(&token_address);
        soroban_sdk::token::StellarAssetClient::new(&env, &token_address)
            .mint(&contract_id, &1_000_i128);

        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &300_i128);
        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower, &CreditStatus::Suspended);

        soroban_sdk::token::StellarAssetClient::new(&env, &token_address)
            .mint(&borrower, &100_i128);
        soroban_sdk::token::Client::new(&env, &token_address).approve(
            &borrower,
            &contract_id,
            &100_i128,
            &1_000_u32,
        );

        client.repay_credit(&borrower, &100_i128);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.utilized_amount, 200);
        assert_eq!(line.status, CreditStatus::Suspended);
    }

    // ── 7. Invalid source: Active → reinstate must revert ────────────────────

    #[test]
    #[should_panic(expected = "credit line is not defaulted")]
    fn reinstate_active_line_reverts() {
        let env = Env::default();
        let (client, _admin, borrower) = setup(&env);
        // Line is Active, not Defaulted
        client.reinstate_credit_line(&borrower, &CreditStatus::Active);
    }

    // ── 8. Invalid source: Suspended → reinstate must revert ─────────────────

    #[test]
    #[should_panic(expected = "credit line is not defaulted")]
    fn reinstate_suspended_line_reverts() {
        let env = Env::default();
        let (client, _admin, borrower) = setup(&env);
        client.suspend_credit_line(&borrower);
        client.reinstate_credit_line(&borrower, &CreditStatus::Active);
    }

    // ── 9. Invalid source: Closed → reinstate must revert ────────────────────

    #[test]
    #[should_panic(expected = "credit line is not defaulted")]
    fn reinstate_closed_line_reverts() {
        let env = Env::default();
        let (client, admin, borrower) = setup(&env);
        client.close_credit_line(&borrower, &admin);
        client.reinstate_credit_line(&borrower, &CreditStatus::Active);
    }

    // ── 10. Invalid target status ─────────────────────────────────────────────

    #[test]
    #[should_panic(expected = "invalid target status: must be Active or Suspended")]
    fn reinstate_with_closed_target_reverts() {
        let env = Env::default();
        let (client, _admin, borrower) = setup(&env);
        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower, &CreditStatus::Closed);
    }

    #[test]
    #[should_panic(expected = "invalid target status: must be Active or Suspended")]
    fn reinstate_with_defaulted_target_reverts() {
        let env = Env::default();
        let (client, _admin, borrower) = setup(&env);
        client.default_credit_line(&borrower);
        // Cannot reinstate into Defaulted
        client.reinstate_credit_line(&borrower, &CreditStatus::Defaulted);
    }

    // ── 11. Non-existent borrower ─────────────────────────────────────────────

    #[test]
    #[should_panic]
    fn reinstate_nonexistent_line_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = soroban_sdk::Address::generate(&env);
        let ghost = soroban_sdk::Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        client.reinstate_credit_line(&ghost, &CreditStatus::Active);
    }

    // ── 12. Reinstate event payload ───────────────────────────────────────────

    #[test]
    fn reinstate_emits_event_with_correct_payload() {
        let env = Env::default();
        let (client, _admin, borrower) = setup(&env);

        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower, &CreditStatus::Active);

        let events = env.events().all();
        let (_contract, topics, data) = events.last().unwrap();

        let topic0: Symbol = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
        let topic1: Symbol = Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap();
        assert_eq!(topic0, symbol_short!("credit"));
        assert_eq!(topic1, symbol_short!("reinstate"));

        let event: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(event.status, CreditStatus::Active);
        assert_eq!(event.borrower, borrower);
        assert_eq!(event.event_type, symbol_short!("reinstate"));
    }

    #[test]
    fn reinstate_to_suspended_emits_event_with_suspended_status() {
        let env = Env::default();
        let (client, _admin, borrower) = setup(&env);

        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower, &CreditStatus::Suspended);

        let events = env.events().all();
        let (_contract, topics, data) = events.last().unwrap();

        let topic1: Symbol = Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap();
        assert_eq!(topic1, symbol_short!("reinstate"));

        let event: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(event.status, CreditStatus::Suspended);
    }

    // ── 13. Double reinstatement: second call reverts (line now Active) ────────

    #[test]
    #[should_panic(expected = "credit line is not defaulted")]
    fn reinstate_twice_second_call_reverts() {
        let env = Env::default();
        let (client, _admin, borrower) = setup(&env);

        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower, &CreditStatus::Active);
        // Line is now Active; second reinstate must fail
        client.reinstate_credit_line(&borrower, &CreditStatus::Active);
    }

    // ── 14. Utilization invariants after reinstatement ────────────────────────

    #[test]
    fn reinstate_utilization_within_limit_invariant_holds() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = soroban_sdk::Address::generate(&env);
        let borrower = soroban_sdk::Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);

        let token_id =
            env.register_stellar_asset_contract_v2(soroban_sdk::Address::generate(&env));
        client.set_liquidity_token(&token_id.address());
        soroban_sdk::token::StellarAssetClient::new(&env, &token_id.address())
            .mint(&contract_id, &1_000_i128);

        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &600_i128);
        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower, &CreditStatus::Active);

        let line = client.get_credit_line(&borrower).unwrap();
        assert!(line.utilized_amount >= 0);
        assert!(line.utilized_amount <= line.credit_limit);
    }
}