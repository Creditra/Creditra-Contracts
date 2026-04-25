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
use crate::events::{publish_credit_line_event, CreditLineEvent};
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
fn liquidation_settlement_key(borrower: &Address, settlement_id: &Symbol) -> (Symbol, Address, Symbol) {
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
        panic!("Only active credit lines can be suspended");
    }

    credit_line.status = CreditStatus::Suspended;
    credit_line.suspension_ts = env.ledger().timestamp();
    env.storage().persistent().set(&borrower, &credit_line);

    publish_credit_line_event(
        env,
        (symbol_short!("credit"), symbol_short!("suspend")),
        CreditLineEvent {
            event_type: symbol_short!("suspend"),
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

    assert!(credit_limit > 0, "credit_limit must be greater than zero");
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
        assert!(
            existing.status != CreditStatus::Active,
            "borrower already has an active credit line"
        );

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
            event_type: symbol_short!("opened"),
            borrower,
            status: CreditStatus::Active,
            credit_limit,
            interest_rate_bps,
            risk_score,
        },
    );
}

/// Suspend an active credit line (admin only).
///
/// # State transition
/// `Active → Suspended`
///
/// # Parameters
/// - `borrower`: The borrower's address.
///
/// # Panics
/// - If no credit line exists for the given borrower.
/// - If the protocol is paused.
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

// ── close_credit_line ─────────────────────────────────────────────────────────

/// Close a credit line (admin force-close, or borrower self-close with zero utilization).
///
/// # State transition
/// `Any non-Closed → Closed`
///
/// # Errors
/// * Panics if credit line does not exist, or if `closer` is not admin/borrower, or if
///   borrower closes while `utilized_amount != 0`, or if the protocol is paused.
///
/// # Errors
/// - Panics with `"unauthorized"` if `closer` is neither admin nor `borrower`.
/// - Panics with `"cannot close: utilized amount not zero"` when borrower
///   tries to close a line with outstanding utilization.
/// - Idempotent: already-Closed lines are accepted silently.
pub fn close_credit_line(env: Env, borrower: Address, closer: Address) {
    assert_not_paused(&env);
    closer.require_auth();

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

    // Apply interest accrual before any mutation
    credit_line = crate::accrual::apply_accrual(&env, credit_line);

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
/// Transitions the credit line to [`CreditStatus::Defaulted`].
///
/// # Valid source statuses
/// - [`CreditStatus::Active`] → Defaulted
/// - [`CreditStatus::Suspended`] → Defaulted
///
/// Closed lines cannot be defaulted (they are permanently closed).
/// Already-Defaulted lines are idempotent (no-op, no event emitted).
///
/// # Effects
/// - `draw_credit` is disabled for the borrower after this call.
/// - `repay_credit` remains allowed so the borrower can reduce their debt.
///
/// # Errors
/// - Panics if the credit line does not exist.
/// - Panics if the caller is not the contract admin.
/// - Panics if the credit line is `Closed`.
///
/// # Events
/// Emits `("credit", "default")` with a [`CreditLineEvent`] payload.
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
        panic!("cannot default a closed credit line");
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
            event_type: symbol_short!("defaulted"),
            borrower: borrower.clone(),
            status: CreditStatus::Defaulted,
            credit_limit: credit_line.credit_limit,
            interest_rate_bps: credit_line.interest_rate_bps,
            risk_score: credit_line.risk_score,
        },
    );

    publish_default_liquidation_requested_event(
        &env,
        DefaultLiquidationRequestedEvent {
            borrower,
            utilized_amount: credit_line.utilized_amount,
            timestamp: env.ledger().timestamp(),
        },
    );
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
        panic!("recovered amount must be positive");
    }

    let settlement_key = liquidation_settlement_key(&borrower, &settlement_id);
    if env.storage().persistent().has(&settlement_key) {
        panic!("liquidation settlement already applied");
    }

    let mut credit_line: CreditLineData = env
        .storage()
        .persistent()
        .get(&borrower)
        .expect("Credit line not found");

    // Apply interest accrual before any mutation
    credit_line = crate::accrual::apply_accrual(&env, credit_line);

    if credit_line.status != CreditStatus::Defaulted {
        panic!("credit line is not defaulted");
    }

    if recovered_amount > credit_line.utilized_amount {
        panic!("recovered amount exceeds utilized amount");
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
                event_type: symbol_short!("closed"),
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
            timestamp: env.ledger().timestamp(),
        },
    );
}

// ── reinstate_credit_line ─────────────────────────────────────────────────────

/// Reinstate a `Defaulted` credit line to either `Active` or `Suspended` (admin only).
///
/// Allowed only when status is Defaulted. Transition: Defaulted → Active.
///
/// # Panics
/// - If the protocol is paused.
pub fn reinstate_credit_line(env: Env, borrower: Address) {
    assert_not_paused(&env);
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

    // Apply interest accrual before any mutation
    credit_line = crate::accrual::apply_accrual(&env, credit_line);

    if credit_line.status != CreditStatus::Defaulted {
        panic!("credit line is not defaulted");
    }

    credit_line.status = CreditStatus::Active;
    credit_line.suspension_ts = 0; // clear grace period anchor on reinstatement
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