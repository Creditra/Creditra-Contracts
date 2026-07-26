// SPDX-License-Identifier: MIT
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]

//! Event types and publishers for the Credit contract.
//!
//! # What
//!
//! Every event the credit contract emits is defined here as a
//! `#[contracttype]` payload struct paired with a `publish_*` helper that
//! calls `env.events().publish((topic_a, topic_b), payload)`.
//!
//! 25+ event topics are published under the `credit` namespace
//! (`("credit","opened")`, `("credit","drawn")`, `("credit","repay")`,
//! `("credit","accrue")`, `("credit","defaulted")`,
//! `("credit","liq_req")`, `("credit","liq_setl")`, etc.) plus the
//! single-element `("blk_chg",)` topic for borrower blocklist changes.
//!
//! **Canonical schema and versioning policy:**
//! See [`docs/events-schema.md`](../../../docs/events-schema.md) for the full
//! authoritative event catalog, topic versions, and payload field orders.
//!
//! # How
//!
//! All topic strings are encoded with `symbol_short!` (≤ 9 characters) so
//! the on-chain encoding is the cheap `SCV_SYMBOL` variant. Payload structs
//! use plain Soroban host types (`Address`, `i128`, `u32`, `u64`,
//! `CreditStatus`) so off-chain indexers can decode them with just the
//! Soroban SDK and the `CreditStatus` discriminant table.
//!
//! # Why (ABI stability)
//!
//! Event topics and payload field layouts are part of the contract's
//! public ABI. The CI test `tests/event_topic_stability.rs` pins every
//! topic string and asserts the payload struct layout has not changed.
//! Breaking changes to the event surface require a new event topic
//! with a version suffix (e.g., `("credit","drawn_v2")`).
//!
//! See [`docs/ARCHITECTURE.md`](../../../docs/ARCHITECTURE.md) for the
//! end-to-end event topology, [`docs/events-schema.md`](../../../docs/events-schema.md)
//! for the canonical catalog and versioning rules, and
//! [`docs/PROTOCOL_SPEC.md`](../../../docs/PROTOCOL_SPEC.md) for the
//! per-entrypoint event-emission table.

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol};

use crate::types::CreditStatus;

/// Dedicated lifecycle event emitted when a credit line is opened.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditLineOpenedEvent {
    pub borrower: Address,
    pub credit_limit: i128,
    pub interest_rate_bps: u32,
    pub risk_score: u32,
    pub timestamp: u64,
}

/// Dedicated lifecycle event emitted when a credit line is suspended.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditLineSuspendedEvent {
    pub borrower: Address,
    pub reason: Symbol,
    pub timestamp: u64,
}

/// Dedicated lifecycle event emitted when a credit line is closed.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditLineClosedEvent {
    pub borrower: Address,
    pub closer: Address,
    pub remaining_utilized_amount: i128,
    pub timestamp: u64,
}

/// Dedicated lifecycle event emitted when a credit line is defaulted.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditLineDefaultedEvent {
    pub borrower: Address,
    pub utilized_amount: i128,
    pub timestamp: u64,
}

/// Dedicated lifecycle event emitted when a credit line is reinstated.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditLineReinstatedEvent {
    pub borrower: Address,
    pub target_status: CreditStatus,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditLineEvent {
    pub borrower: Address,
    pub status: CreditStatus,
    pub credit_limit: i128,
    pub interest_rate_bps: u32,
    pub risk_score: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepaymentEvent {
    pub borrower: Address,
    pub amount: i128,
    pub new_utilized_amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DrawnEvent {
    pub borrower: Address,
    pub amount: i128,
    pub new_utilized_amount: i128,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterestAccruedEvent {
    pub borrower: Address,
    pub accrued_amount: i128,
    pub new_utilized_amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DefaultLiquidationSettledEvent {
    pub borrower: Address,
    pub settlement_id: Symbol,
    pub recovered_amount: i128,
    pub remaining_utilized_amount: i128,
    pub status: CreditStatus,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminRotationProposedEvent {
    pub proposed_admin: Address,
    pub accept_after: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminRotationAcceptedEvent {
    pub new_admin: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskParametersUpdatedEvent {
    pub borrower: Address,
    pub credit_limit: i128,
    pub interest_rate_bps: u32,
    pub risk_score: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DrawReversedEvent {
    pub borrower: Address,
    pub amount: i128,
    pub original_ts: u64,
    pub reason_code: u32,
    pub new_utilized_amount: i128,
    pub timestamp: u64,
    pub admin: Address,
    pub accounting_only: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DrawsFrozenEvent {
    pub frozen: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BorrowerBlockedEvent {
    pub borrower: Address,
    pub blocked: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DrawnEventV2 {
    pub borrower: Address,
    pub recipient: Address,
    pub reserve_source: Address,
    pub amount: i128,
    pub new_utilized_amount: i128,
    pub timestamp: u64,
}

pub fn publish_credit_line_opened_event(env: &Env, event: CreditLineOpenedEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("opened_v2")), event);
}

pub fn publish_credit_line_suspended_event(env: &Env, event: CreditLineSuspendedEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("susp_v2")), event);
}

pub fn publish_credit_line_closed_event(env: &Env, event: CreditLineClosedEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("closed_v2")), event);
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraceWaiverAppliedEvent {
    pub borrower: Address,
    pub waived_amount: i128,
    pub mode: crate::types::GraceWaiverMode,
}

pub fn publish_credit_line_event(env: &Env, topic: (Symbol, Symbol), event: CreditLineEvent) {
    env.events().publish(topic, event);
}

pub fn publish_repayment_event(env: &Env, event: RepaymentEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("repay")), event);
}

pub fn publish_drawn_event(env: &Env, event: DrawnEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("drawn")), event);
}

/// Publish a draw reversal event.
pub fn publish_draw_reversed_event(env: &Env, event: DrawReversedEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("draw_rev")), event);
}

/// Publish a v2 drawn event.
#[allow(dead_code)]
pub fn publish_drawn_event_v2(env: &Env, event: DrawnEventV2) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("drawn_v2")), event);
}

pub fn publish_fee_accrued_event(env: &Env, event: FeeAccruedEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("fee_accrd")), event);
}

pub fn publish_admin_rotation_proposed(env: &Env, proposed_admin: &Address, accept_after: u64) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "admin_prop")),
        AdminRotationProposedEvent {
            proposed_admin: proposed_admin.clone(),
            accept_after,
        },
    );
}

pub fn publish_admin_rotation_accepted(env: &Env, new_admin: &Address) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "admin_acc")),
        AdminRotationAcceptedEvent {
            new_admin: new_admin.clone(),
        },
    );
}

pub fn publish_risk_parameters_updated(
    env: &Env,
    borrower: &Address,
    credit_limit: i128,
    interest_rate_bps: u32,
    risk_score: u32,
) {
    env.events().publish(
        (symbol_short!("credit"), symbol_short!("risk_upd")),
        RiskParametersUpdatedEvent {
            borrower: borrower.clone(),
            credit_limit,
            interest_rate_bps,
            risk_score,
        },
    );
}

pub fn publish_interest_accrued_event(env: &Env, event: InterestAccruedEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("accrue")), event);
}

pub fn publish_draws_frozen_event(env: &Env, frozen: bool) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "drw_freeze")),
        DrawsFrozenEvent { frozen },
    );
}

pub fn publish_rate_formula_config_event(env: &Env, enabled: bool) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "rate_form")),
        enabled,
    );
}

pub fn publish_default_liquidation_requested_event(
    env: &Env,
    borrower: &Address,
    utilized_amount: i128,
) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "liq_req")),
        (borrower.clone(), utilized_amount),
    );
}

pub fn publish_default_liquidation_settled_event(
    env: &Env,
    event: DefaultLiquidationSettledEvent,
) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "liq_setl")),
        event,
    );
}

pub fn publish_paused_event(env: &Env, paused: bool) {
    let topic = if paused {
        Symbol::new(env, "paused")
    } else {
        Symbol::new(env, "unpaused")
    };
    env.events().publish((symbol_short!("credit"), topic), paused);
}

/// Publish a borrower blocked/unblocked event.
pub fn publish_borrower_blocked_event(env: &Env, borrower: &Address, blocked: bool) {
    env.events().publish(
        (Symbol::new(env, "blk_chg"),),
        BorrowerBlockedEvent {
            borrower: borrower.clone(),
            blocked,
            ledger: env.ledger().sequence(),
        },
    );
}

/// Publish a borrower temporary freeze event.
///
/// Emitted when an admin sets a time-bounded freeze on a borrower's draws.
/// The `frozen_until` field records the ledger timestamp at which the freeze
/// will auto-expire.
///
/// # Topic
/// `("credit", "brw_frz")`
pub fn publish_borrower_frozen_event(env: &Env, borrower: &Address, frozen_until: u64) {
    env.events().publish(
        (Symbol::new(env, "br_freeze"),),
        BorrowerFrozenEvent {
            borrower: borrower.clone(),
            frozen_until,
            ledger: env.ledger().sequence(),
        },
    );
}

/// Publish a penalty rate entered event when a line becomes delinquent.

/// Publish a penalty rate entered event when a line becomes delinquent.
pub fn publish_penalty_rate_entered_event(
    env: &Env,
    borrower: &Address,
    base_rate_bps: u32,
    penalty_surcharge_bps: u32,
    effective_rate_bps: u32,
) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "pen_enter")),
        PenaltyRateEnteredEvent {
            borrower: borrower.clone(),
            base_rate_bps,
            penalty_surcharge_bps,
            effective_rate_bps,
        },
    );
}

/// Publish a penalty rate exited event when a line is no longer delinquent.
pub fn publish_penalty_rate_exited_event(
    env: &Env,
    borrower: &Address,
    previous_rate_bps: u32,
    new_rate_bps: u32,
) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "pen_exit")),
        PenaltyRateExitedEvent {
            borrower: borrower.clone(),
            previous_rate_bps,
            new_rate_bps,
        },
    );
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollateralDepositedEvent {
    pub borrower: Address,
    pub amount: i128,
    pub new_balance: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollateralWithdrawnEvent {
    pub borrower: Address,
    pub amount: i128,
    pub new_balance: i128,
}

/// Structured, unified lifecycle event covering every collateral state
/// change (deposit, withdrawal, partial release, internal release).
///
/// Emitted **in addition to** the legacy per-action events
/// ([`CollateralDepositedEvent`], [`CollateralWithdrawnEvent`],
/// [`CollateralPartialReleasedEvent`]) so existing indexers keep working
/// unmodified, while new integrators can subscribe to a single topic
/// (`("credit", "col_lca")`) and disambiguate via [`crate::types::CollateralEventKind`]
/// instead of tracking multiple topic strings.
///
/// # Field notes
///
/// - `token` is `None` for the single-token collateral path (`CollateralBalance`
///   storage) and `Some(token)` for the multi-collateral, per-token path.
/// - `ledger` / `timestamp` let off-chain consumers order and correlate
///   events without a separate RPC round-trip to fetch ledger metadata.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollateralLifecycleEvent {
    pub borrower: Address,
    pub kind: crate::types::CollateralEventKind,
    /// `None` for the single-token collateral balance; `Some(token)` for
    /// the multi-collateral per-token path.
    pub token: Option<Address>,
    /// Amount moved by this action (always positive).
    pub amount: i128,
    /// Collateral balance remaining after this action.
    pub new_balance: i128,
    /// Ledger sequence at time of the action (for off-chain indexers).
    pub ledger: u32,
    /// Ledger timestamp at time of the action.
    pub timestamp: u64,
}

/// Publish a [`CollateralLifecycleEvent`] under the unified `("credit", "col_lca")` topic.
pub fn publish_collateral_lifecycle_event(
    env: &Env,
    borrower: &Address,
    kind: crate::types::CollateralEventKind,
    token: Option<Address>,
    amount: i128,
    new_balance: i128,
) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "col_lca")),
        CollateralLifecycleEvent {
            borrower: borrower.clone(),
            kind,
            token,
            amount,
            new_balance,
            ledger: env.ledger().sequence(),
            timestamp: env.ledger().timestamp(),
        },
    );
}

pub fn publish_collateral_deposited_event(env: &Env, event: CollateralDepositedEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("col_dep")), event);
}

pub fn publish_collateral_withdrawn_event(env: &Env, event: CollateralWithdrawnEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("col_wit")), event);
}
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenRescuedEvent {
    pub token: Address,
    pub recipient: Address,
    pub amount: i128,
}

pub fn publish_token_rescued_event(env: &Env, event: TokenRescuedEvent) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "tok_resc")),
        event,
    );
}
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContractUpgradedEvent {
    pub old_wasm_hash: soroban_sdk::BytesN<32>,
    pub new_wasm_hash: soroban_sdk::BytesN<32>,
}

pub fn publish_contract_upgraded_event(env: &Env, event: ContractUpgradedEvent) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "upgraded")),
        event,
    );
}

pub fn publish_close_factor_bps_set_event(env: &Env, close_factor_bps: u32) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "clsfctr")),
        close_factor_bps,
    );
}

pub fn publish_oracle_config_set_event(env: &Env, max_deviation_bps: u32, max_age_seconds: u64) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "orc_cfg")),
        (max_deviation_bps, max_age_seconds),
    );
}

pub fn publish_oracle_price_accepted_event(env: &Env, price: i128, timestamp: u64) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "orc_price")),
        (price, timestamp),
    );
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LateFeeChargedEvent {
    pub borrower: Address,
    pub fee: i128,
    pub installment_index: u64,
}

/// Publish a late fee charged event when a missed installment is detected.
pub fn publish_late_fee_charged_event(env: &Env, event: LateFeeChargedEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("late_fee")), event);
}

/// Publish a grace waiver applied event when a suspended line's accrual uses the grace period.
pub fn publish_grace_waiver_applied_event(
    env: &Env,
    borrower: &Address,
    waived_amount: i128,
    mode: crate::types::GraceWaiverMode,
) {
    env.events().publish(
        (symbol_short!("credit"), symbol_short!("grace_wv")),
        GraceWaiverAppliedEvent {
            borrower: borrower.clone(),
            waived_amount,
            mode,
        },
    );
}
