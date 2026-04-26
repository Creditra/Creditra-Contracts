// SPDX-License-Identifier: MIT

//! Credit line lifecycle management: suspend, close, default, reinstate, and liquidation settlement.
//!
//! # Storage
//! - **Borrower credit lines**: Persistent storage (independent TTL per borrower)
//!   - Key: `borrower: Address`
//!   - Value: `CreditLineData`
//! - **Liquidation settlement markers**: Persistent storage (replay protection)
//!   - Key: `(Symbol("liq_seen"), borrower, settlement_id)`
//!   - Value: `bool`

use crate::auth::{require_admin, require_admin_auth};
use crate::events::{
    publish_credit_line_event, publish_default_liquidation_requested_event,
    publish_default_liquidation_settled_event, CreditLineEvent, DefaultLiquidationSettledEvent,
};
use crate::risk::{MAX_INTEREST_RATE_BPS, MAX_RISK_SCORE};
use crate::storage::assert_not_paused;
use crate::types::{ContractError, CreditLineData, CreditStatus};
use soroban_sdk::{symbol_short, Address, Env, Symbol};

/// Generate a unique key for tracking liquidation settlements.
///
/// # Storage
/// - **Type**: Persistent storage (independent TTL per settlement)
/// - **Key**: `(Symbol("liq_seen"), borrower, settlement_id)`
/// - **Purpose**: Prevents replay of the same liquidation settlement
fn liquidation_settlement_key(
    borrower: &Address,
    settlement_id: &Symbol,
) -> (Symbol, Address, Symbol) {
    (
        symbol_short!("liq_seen"),
        borrower.clone(),
        settlement_id.clone(),
    )
}

fn suspend_credit_line_internal(env: &Env, borrower: Address) {
    let mut credit_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .expect("Credit line not found");

    // Apply interest accrual before any mutation.
    credit_line = crate::accrual::apply_accrual(env, credit_line);

    if credit_line.status != CreditStatus::Active {
        env.panic_with_error(ContractError::CreditLineSuspended);
    }

    credit_line.status = CreditStatus::Suspended;
    credit_line.suspension_ts = env.ledger().timestamp();
    env.storage().persistent().set(&borrower, &credit_line);

    publish_credit_line_event(
        env,
        (symbol_short!("credit"), symbol_short!("suspend")),
        CreditLineEvent {
            borrower,
            status: CreditStatus::Suspended,
            credit_limit: credit_line.credit_limit,
            interest_rate_bps: credit_line.interest_rate_bps,
            risk_score: credit_line.risk_score,
        },
    );
}

/// Open a new credit line.
///
/// Creating a brand-new line preserves the existing backend/risk-engine trust
/// boundary. Re-opening any existing non-Active line requires admin auth so a
/// borrower cannot self-suspend and then reactivate themselves on-chain.
pub fn open_credit_line(
    env: Env,
    borrower: Address,
    credit_limit: i128,
    interest_rate_bps: u32,
    risk_score: u32,
) {
    assert_not_paused(&env);

    if credit_limit <= 0 {
        env.panic_with_error(ContractError::InvalidAmount);
    }
    if interest_rate_bps > MAX_INTEREST_RATE_BPS {
        env.panic_with_error(ContractError::RateTooHigh);
    }
    if risk_score > MAX_RISK_SCORE {
        env.panic_with_error(ContractError::ScoreTooHigh);
    }

    if let Some(existing) = env
        .storage()
        .persistent()
        .get::<Address, CreditLineData>(&borrower)
    {
        if existing.status == CreditStatus::Active {
            env.panic_with_error(ContractError::AlreadyInitialized);
        }

        // Prevent borrower-controlled status bypasses on existing lines.
        require_admin_auth(&env);
    }

    let credit_line = CreditLineData {
        borrower: borrower.clone(),
        credit_limit,
        utilized_amount: 0,
        interest_rate_bps,
        risk_score,
        status: CreditStatus::Active,
        last_rate_update_ts: 0,
        accrued_interest: 0,
        last_accrual_ts: env.ledger().timestamp(),
        suspension_ts: 0,
    };
    env.storage().persistent().set(&borrower, &credit_line);

    publish_credit_line_event(
        &env,
        (symbol_short!("credit"), symbol_short!("opened")),
        CreditLineEvent {
            borrower,
            status: CreditStatus::Active,
            credit_limit,
            interest_rate_bps,
            risk_score,
        },
    );
}

/// Suspend a credit line temporarily (admin only).
///
/// # State transition
/// `Active → Suspended`
///
/// # Parameters
/// - `borrower`: The borrower's address.
///
/// # Panics
/// - If no credit line exists for the given borrower.
/// - If the credit line is not currently `Active`.
///
/// # Events
/// Emits a `("credit", "suspend")` [`CreditLineEvent`].
pub fn suspend_credit_line(env: Env, borrower: Address) {
    assert_not_paused(&env);
    require_admin_auth(&env);
    suspend_credit_line_internal(&env, borrower);
}

/// Suspend the caller's own active credit line.
///
/// This is a borrower safety control that blocks future draws while leaving
/// repayments available. Reactivation still requires a separate admin-controlled
/// workflow.
pub fn self_suspend_credit_line(env: Env, borrower: Address) {
    assert_not_paused(&env);
    borrower.require_auth();
    suspend_credit_line_internal(&env, borrower);
}

/// Close a credit line permanently.
///
/// Transitions the credit line to [`CreditStatus::Closed`]. Once closed, no further draws or
/// repayments are permitted. A closed line can be replaced by a new [`open_credit_line`] call.
///
/// # Authorization rules
///
/// | `closer` identity | Condition to close |
/// |-------------------|--------------------|
/// | Admin             | Always allowed, regardless of `utilized_amount` or current status |
/// | Borrower          | Allowed only when `utilized_amount == 0` |
/// | Any other address | Always rejected with `"unauthorized"` |
///
/// # Idempotency
/// If the credit line is already [`CreditStatus::Closed`], the call returns without error or
/// event. This makes the function safe to call defensively (e.g., in cleanup workflows).
///
/// # Parameters
/// - `borrower`: Address whose credit line is being closed.
/// - `closer`:   Address authorizing the close. Must be the admin or the borrower.
///
/// # Panics
/// - `"Credit line not found"` — no credit line exists for `borrower`.
/// - `"cannot close: utilized amount not zero"` — `closer == borrower` but outstanding balance > 0.
/// - `"unauthorized"` — `closer` is neither the admin nor the borrower.
///
/// # Events
/// Emits a `("credit", "closed")` [`CreditLineEvent`] on successful state change.
/// No event is emitted when the line is already closed (idempotent path).
///
/// # Security notes
/// - `closer.require_auth()` is called before any storage reads, so an unauthenticated
///   call is rejected at the Soroban host level before any state is inspected.
/// - The authorization check uses address equality against the stored admin and the
///   `borrower` parameter — there is no privileged role beyond these two identities.
/// - Closing does **not** require prior suspension or default; admin can force-close from any
///   non-closed status. This is intentional for operational efficiency.
pub fn close_credit_line(env: Env, borrower: Address, closer: Address) {
    // Authenticate the closer before any storage access.
    closer.require_auth();

    // Resolve the current admin address.
    let admin: Address = require_admin(&env);

    // Load the credit line; revert if it does not exist.
    let mut credit_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .expect("Credit line not found");

    // Idempotent: already closed → nothing to do.
    if credit_line.status == CreditStatus::Closed {
        return;
    }

    // Authorization: determine whether `closer` is permitted to close this line.
    //
    // Three mutually exclusive cases, checked in priority order:
    //   1. closer == admin           → always permitted (force-close).
    //   2. closer == borrower        → permitted only when utilization is zero.
    //   3. closer is someone else    → always rejected.
    if closer == admin {
        // Admin force-close: no utilization restriction.
    } else if closer == borrower {
        // Borrower self-close: only allowed when fully repaid.
        if credit_line.utilized_amount != 0 {
            panic!("cannot close: utilized amount not zero");
        }
    } else {
        // Third party: unconditionally rejected.
        panic!("unauthorized");
    }

    credit_line.status = CreditStatus::Closed;
    env.storage().persistent().set(&borrower, &credit_line);

    publish_credit_line_event(
        &env,
        (symbol_short!("credit"), symbol_short!("closed")),
        CreditLineEvent {
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
/// Transition: `Active` or `Suspended` → `Defaulted`.
/// After defaulting, `draw_credit` is disabled and `repay_credit` remains allowed.
///
/// # Events
/// Emits a `("credit", "default")` [`CreditLineEvent`].
pub fn default_credit_line(env: Env, borrower: Address) {
    assert_not_paused(&env);
    require_admin_auth(&env);
    let mut credit_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));

    if credit_line.status == CreditStatus::Closed {
        env.panic_with_error(ContractError::CreditLineClosed);
    }

    // Apply interest accrual before any mutation
    credit_line = crate::accrual::apply_accrual(&env, credit_line);

    if credit_line.status == CreditStatus::Closed {
        env.panic_with_error(ContractError::CreditLineClosed);
    }

    if credit_line.status == CreditStatus::Defaulted {
        // Idempotent: already defaulted, nothing to do.
        return;
    }

    credit_line.status = CreditStatus::Defaulted;
    env.storage().persistent().set(&borrower, &credit_line);

    publish_credit_line_event(
        &env,
        (symbol_short!("credit"), symbol_short!("defaulted")),
        CreditLineEvent {
            borrower: borrower.clone(),
            status: CreditStatus::Defaulted,
            credit_limit: credit_line.credit_limit,
            interest_rate_bps: credit_line.interest_rate_bps,
            risk_score: credit_line.risk_score,
        },
    );

    publish_default_liquidation_requested_event(&env, &borrower, credit_line.utilized_amount);
}

/// Apply auction liquidation proceeds to a defaulted credit line (admin only).
///
/// This hook is accounting-only and intentionally performs no token transfer.
/// Off-chain orchestration is responsible for ensuring auction proceeds are settled
/// into protocol custody before this function is called.
pub fn settle_default_liquidation(
    env: Env,
    borrower: Address,
    recovered_amount: i128,
    settlement_id: Symbol,
) {
    require_admin_auth(&env);

    if recovered_amount <= 0 {
        env.panic_with_error(ContractError::InvalidAmount);
    }

    let settlement_key = liquidation_settlement_key(&borrower, &settlement_id);
    if env.storage().persistent().has(&settlement_key) {
        env.panic_with_error(ContractError::AlreadyInitialized); // Or a specific LiquidationAlreadyApplied
    }

    let mut credit_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .expect("Credit line not found");

    // Apply interest accrual before any mutation
    credit_line = crate::accrual::apply_accrual(&env, credit_line);

    if credit_line.status != CreditStatus::Defaulted {
        env.panic_with_error(ContractError::CreditLineDefaulted);
    }

    if recovered_amount > credit_line.utilized_amount {
        env.panic_with_error(ContractError::OverLimit); // Or a specific error
    }

    credit_line.utilized_amount = credit_line
        .utilized_amount
        .checked_sub(recovered_amount)
        .expect("overflow while applying liquidation settlement");

    if credit_line.utilized_amount == 0 {
        credit_line.status = CreditStatus::Closed;
    }

    env.storage().persistent().set(&borrower, &credit_line);
    env.storage().persistent().set(&settlement_key, &true);

    if credit_line.status == CreditStatus::Closed {
        publish_credit_line_event(
            &env,
            (symbol_short!("credit"), symbol_short!("closed")),
            CreditLineEvent {
                borrower: borrower.clone(),
                status: CreditStatus::Closed,
                credit_limit: credit_line.credit_limit,
                interest_rate_bps: credit_line.interest_rate_bps,
                risk_score: credit_line.risk_score,
            },
        );
    }

    publish_default_liquidation_settled_event(
        &env,
        DefaultLiquidationSettledEvent {
            borrower,
            settlement_id,
            recovered_amount,
            remaining_utilized_amount: credit_line.utilized_amount,
            status: credit_line.status,
        },
    );
}

// ── reinstate_credit_line ─────────────────────────────────────────────────────

/// Reinstate a `Defaulted` credit line to either `Active` or `Suspended` (admin only).
///
/// Allowed only when current status is `Defaulted`. Transition: `Defaulted` → `Active`.
///
/// # Panics
/// - `"Credit line not found"` — no credit line exists for `borrower`.
/// - `"credit line is not defaulted"` — current status is not `Defaulted`.
///
/// # Events
/// Emits a `("credit", "reinstate")` [`CreditLineEvent`].
pub fn reinstate_credit_line(env: Env, borrower: Address) {
    assert_not_paused(&env);
    require_admin_auth(&env);

    if target_status != CreditStatus::Active && target_status != CreditStatus::Suspended {
        env.panic_with_error(ContractError::InvalidAmount);
    }

    let mut credit_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));

    credit_line = crate::accrual::apply_accrual(&env, credit_line);

    if credit_line.status != CreditStatus::Defaulted {
        env.panic_with_error(ContractError::CreditLineDefaulted);
    }

    credit_line.status = target_status;
    credit_line.suspension_ts = 0;
    env.storage().persistent().set(&borrower, &credit_line);

    publish_credit_line_event(
        &env,
        (symbol_short!("credit"), symbol_short!("reinstate")),
        CreditLineEvent {
            borrower: borrower.clone(),
            status: target_status,
            credit_limit: credit_line.credit_limit,
            interest_rate_bps: credit_line.interest_rate_bps,
            risk_score: credit_line.risk_score,
        },
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: close_credit_line authorization and utilization rules (#228)
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod test_close_credit_line {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::testutils::Events as _;
    use soroban_sdk::{symbol_short, Symbol, TryFromVal, TryIntoVal};
    use soroban_sdk::{Address, Env};

    // Minimal in-module contract stub so tests can call the contract client without
    // importing the full lib.rs (which has duplicate-mod issues in the upstream file).
    // We test `close_credit_line` by calling lifecycle functions directly via a
    // thin wrapper contract registered in the test environment.

    use crate::storage::DataKey;
    use soroban_sdk::{contract, contractimpl};

    #[contract]
    struct TestCredit;

    #[contractimpl]
    impl TestCredit {
        pub fn init(env: Env, admin: Address) {
            let key = crate::storage::admin_key(&env);
            env.storage().instance().set(&key, &admin);
            env.storage()
                .instance()
                .set(&DataKey::LiquiditySource, &env.current_contract_address());
        }

        pub fn open(
            env: Env,
            borrower: Address,
            credit_limit: i128,
            interest_rate_bps: u32,
            risk_score: u32,
        ) {
            open_credit_line(env, borrower, credit_limit, interest_rate_bps, risk_score);
        }

        pub fn draw(env: Env, borrower: Address, amount: i128) {
            // Minimal draw: just mutate storage so we can test closing with utilization.
            borrower.require_auth();
            let mut line: CreditLineData = env
                .storage()
                .persistent()
                .get(&borrower)
                .expect("not found");
            line.utilized_amount += amount;
            env.storage().persistent().set(&borrower, &line);
        }

        pub fn close(env: Env, borrower: Address, closer: Address) {
            close_credit_line(env, borrower, closer);
        }

        pub fn suspend(env: Env, borrower: Address) {
            suspend_credit_line(env, borrower);
        }

        pub fn default_line(env: Env, borrower: Address) {
            default_credit_line(env, borrower);
        }

        pub fn reinstate(env: Env, borrower: Address) {
            reinstate_credit_line(env, borrower);
        }

        pub fn get(env: Env, borrower: Address) -> Option<CreditLineData> {
            env.storage().persistent().get(&borrower)
        }
    }

    // ── helpers ───────────────────────────────────────────────────────────────

    fn setup(env: &Env) -> (TestCreditClient<'_>, Address, Address) {
        env.mock_all_auths();
        let admin = Address::generate(env);
        let contract_id = env.register(TestCredit, ());
        let client = TestCreditClient::new(env, &contract_id);
        client.init(&admin);
        (client, contract_id, admin)
    }

    fn open_line(client: &TestCreditClient<'_>, borrower: &Address) {
        client.open(borrower, &1_000_i128, &300_u32, &70_u32);
    }

    // ── 1. Borrower closes with zero utilization ───────────────────────────────

    #[test]
    fn borrower_can_close_when_utilization_is_zero() {
        let env = Env::default();
        let (client, _cid, _admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);

        // utilized_amount is 0 at open → borrower can close
        client.close(&borrower, &borrower);

        let line = client.get(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Closed);
        assert_eq!(line.utilized_amount, 0);
    }

    // ── 2. Admin closes with non-zero utilization (force-close) ───────────────

    #[test]
    fn admin_can_force_close_with_non_zero_utilization() {
        let env = Env::default();
        let (client, _cid, admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);
        client.draw(&borrower, &400_i128);

        assert_eq!(client.get(&borrower).unwrap().utilized_amount, 400);

        client.close(&borrower, &admin);

        let line = client.get(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Closed);
    }

    // ── 3. Admin closes with zero utilization ────────────────────────────────

    #[test]
    fn admin_can_close_with_zero_utilization() {
        let env = Env::default();
        let (client, _cid, admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);

        client.close(&borrower, &admin);

        assert_eq!(
            client.get(&borrower).unwrap().status,
            CreditStatus::Closed
        );
    }

    // ── 4. Borrower cannot close with outstanding balance ─────────────────────

    #[test]
    #[should_panic(expected = "cannot close: utilized amount not zero")]
    fn borrower_cannot_close_with_non_zero_utilization() {
        let env = Env::default();
        let (client, _cid, _admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);
        client.draw(&borrower, &1_i128); // any positive draw

        client.close(&borrower, &borrower);
    }

    // ── 5. Third party (neither admin nor borrower) is rejected ───────────────

    #[test]
    #[should_panic(expected = "unauthorized")]
    fn stranger_cannot_close_credit_line() {
        let env = Env::default();
        let (client, _cid, _admin) = setup(&env);
        let borrower = Address::generate(&env);
        let stranger = Address::generate(&env);
        open_line(&client, &borrower);

        client.close(&borrower, &stranger);
    }

    // ── 6. Stranger with zero utilization is still rejected ───────────────────

    #[test]
    #[should_panic(expected = "unauthorized")]
    fn stranger_cannot_close_even_with_zero_utilization() {
        let env = Env::default();
        let (client, _cid, _admin) = setup(&env);
        let borrower = Address::generate(&env);
        let stranger = Address::generate(&env);
        open_line(&client, &borrower);
        // line has zero utilization but closer is neither admin nor borrower
        client.close(&borrower, &stranger);
    }

    // ── 7. Close is idempotent when already Closed ────────────────────────────

    #[test]
    fn close_is_idempotent_when_already_closed() {
        let env = Env::default();
        let (client, _cid, admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);

        client.close(&borrower, &admin);
        // Second call must not panic
        client.close(&borrower, &admin);

        assert_eq!(
            client.get(&borrower).unwrap().status,
            CreditStatus::Closed
        );
    }

    // ── 8. No draw after close ────────────────────────────────────────────────
    // (draw is tested at the lib.rs level via draw_credit; here we verify that
    //  storage status is Closed so the draw_credit status check will fire.)

    #[test]
    fn closed_line_has_closed_status_preventing_draws() {
        let env = Env::default();
        let (client, _cid, admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);
        client.close(&borrower, &admin);

        let line = client.get(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Closed);
        // draw_credit in lib.rs checks status == Closed and reverts with CreditLineClosed
    }

    // ── 9. Admin closes a Suspended line ─────────────────────────────────────

    #[test]
    fn admin_can_close_suspended_line() {
        let env = Env::default();
        let (client, _cid, admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);
        client.suspend(&borrower);

        assert_eq!(
            client.get(&borrower).unwrap().status,
            CreditStatus::Suspended
        );

        client.close(&borrower, &admin);

        assert_eq!(
            client.get(&borrower).unwrap().status,
            CreditStatus::Closed
        );
    }

    // ── 10. Admin closes a Defaulted line ────────────────────────────────────

    #[test]
    fn admin_can_close_defaulted_line() {
        let env = Env::default();
        let (client, _cid, admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);
        client.default_line(&borrower);

        assert_eq!(
            client.get(&borrower).unwrap().status,
            CreditStatus::Defaulted
        );

        client.close(&borrower, &admin);

        assert_eq!(
            client.get(&borrower).unwrap().status,
            CreditStatus::Closed
        );
    }

    // ── 11. Borrower closes a Suspended line with zero utilization ────────────

    #[test]
    fn borrower_can_close_suspended_line_with_zero_utilization() {
        let env = Env::default();
        let (client, _cid, _admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);
        client.suspend(&borrower);

        // utilized_amount is still 0 → borrower may close
        client.close(&borrower, &borrower);

        assert_eq!(
            client.get(&borrower).unwrap().status,
            CreditStatus::Closed
        );
    }

    // ── 12. close emits ("credit", "closed") event ────────────────────────────

    #[test]
    fn close_emits_closed_event_with_correct_topics_and_status() {
        let env = Env::default();
        let (client, _cid, admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);

        client.close(&borrower, &admin);

        let events = env.events().all();
        let (_contract, topics, data) = events.last().unwrap();

        let topic0: Symbol = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
        let topic1: Symbol = Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap();
        assert_eq!(topic0, symbol_short!("credit"));
        assert_eq!(topic1, symbol_short!("closed"));

        let event: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(event.status, CreditStatus::Closed);
        assert_eq!(event.borrower, borrower);
    }

    // ── 13. Idempotent close emits no second event ────────────────────────────

    #[test]
    fn idempotent_close_emits_no_additional_event() {
        let env = Env::default();
        let (client, _cid, admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);

        client.close(&borrower, &admin);
        let event_count_after_first = env.events().all().len();

        client.close(&borrower, &admin); // idempotent
        let event_count_after_second = env.events().all().len();

        assert_eq!(
            event_count_after_first, event_count_after_second,
            "idempotent close must not emit a second event"
        );
    }

    // ── 14. Non-existent credit line reverts ─────────────────────────────────

    #[test]
    #[should_panic(expected = "Credit line not found")]
    fn close_nonexistent_line_reverts() {
        let env = Env::default();
        let (client, _cid, admin) = setup(&env);
        let borrower = Address::generate(&env); // no open_line call

        client.close(&borrower, &admin);
    }

    // ── 15. Closed line status persists; other fields unchanged ───────────────

    #[test]
    fn close_sets_status_to_closed_and_does_not_mutate_other_fields() {
        let env = Env::default();
        let (client, _cid, admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);
        let before = client.get(&borrower).unwrap();

        client.close(&borrower, &admin);
        let after = client.get(&borrower).unwrap();

        assert_eq!(after.status, CreditStatus::Closed);
        assert_eq!(after.borrower, before.borrower);
        assert_eq!(after.credit_limit, before.credit_limit);
        assert_eq!(after.utilized_amount, before.utilized_amount);
        assert_eq!(after.interest_rate_bps, before.interest_rate_bps);
        assert_eq!(after.risk_score, before.risk_score);
    }

    // ── 16. open_credit_line succeeds after Closed (re-open path) ─────────────

    #[test]
    fn open_credit_line_succeeds_after_close() {
        let env = Env::default();
        let (client, _cid, admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);
        client.close(&borrower, &admin);

        // Re-opening a Closed line must succeed (status != Active guard)
        client.open(&borrower, &2_000_i128, &400_u32, &60_u32);

        let line = client.get(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Active);
        assert_eq!(line.credit_limit, 2_000);
        assert_eq!(line.utilized_amount, 0);
    }

    // ── 17. Borrower closes with exact-zero boundary ──────────────────────────

    #[test]
    fn borrower_close_at_exact_zero_utilization_boundary() {
        let env = Env::default();
        let (client, _cid, _admin) = setup(&env);
        let borrower = Address::generate(&env);

        // Open with credit_limit == 1 to make the boundary obvious
        client.open(&borrower, &1_i128, &300_u32, &70_u32);
        // Do not draw; utilized_amount == 0 exactly
        client.close(&borrower, &borrower);

        assert_eq!(
            client.get(&borrower).unwrap().status,
            CreditStatus::Closed
        );
    }

    // ── 18. Admin auth is required ────────────────────────────────────────────

    #[test]
    fn close_records_closer_auth_requirement() {
        let env = Env::default();
        let (client, _cid, admin) = setup(&env);
        let borrower = Address::generate(&env);
        open_line(&client, &borrower);

        client.close(&borrower, &admin);

        // Verify that the admin address was required to authenticate
        assert!(
            env.auths().iter().any(|(addr, _)| *addr == admin),
            "close_credit_line must require closer authorization"
        );
    }
}
