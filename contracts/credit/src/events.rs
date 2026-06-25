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
//! The full catalog is in
//! [`docs/indexer-integration.md`](../../../docs/indexer-integration.md).
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
//! Breaking changes to the event surface require a major version bump in
//! [`crate::CONTRACT_API_VERSION`].
//!
//! Additive evolution is safe:
//!
//! - New event topics are safe to add (existing topics keep working).
//! - New fields on an existing payload struct can be added at the end
//!   *only* via a new event topic with a versioned suffix (e.g.
//!   `("credit","drawn_v2")`, which already appears here as
//!   [`DrawnEventV2`] reserved for the next event-schema iteration).
//!
//! See [`docs/ARCHITECTURE.md`](../../../docs/ARCHITECTURE.md) for the
//! end-to-end event topology and
//! [`docs/PROTOCOL_SPEC.md`](../../../docs/PROTOCOL_SPEC.md) for the
//! per-entrypoint event-emission table.

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol};

use crate::types::CreditStatus;

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
    /// true = borrower was blocked; false = borrower was unblocked
    pub blocked: bool,
    /// Ledger sequence at time of change (for off-chain indexers)
    pub ledger: u32,
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

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeAccruedEvent {
    pub borrower: Address,
    pub fee_amount: i128,
    pub new_treasury_balance: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PenaltyRateEnteredEvent {
    pub borrower: Address,
    pub base_rate_bps: u32,
    pub penalty_surcharge_bps: u32,
    pub effective_rate_bps: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PenaltyRateExitedEvent {
    pub borrower: Address,
    pub previous_rate_bps: u32,
    pub new_rate_bps: u32,
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

pub fn publish_default_liquidation_settled_event(env: &Env, event: DefaultLiquidationSettledEvent) {
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
    env.events()
        .publish((symbol_short!("credit"), topic), paused);
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
