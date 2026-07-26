// SPDX-License-Identifier: MIT
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]

//! Event types and publishers for the Credit contract.

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

pub fn publish_credit_line_defaulted_event(env: &Env, event: CreditLineDefaultedEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("def_v2")), event);
}

pub fn publish_credit_line_reinstated_event(env: &Env, event: CreditLineReinstatedEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("rein_v2")), event);
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
#[allow(dead_code)]
pub fn publish_borrower_blocked_event(env: &Env, event: BorrowerBlockedEvent) {
    env.events()
        .publish((symbol_short!("credit"), symbol_short!("col_dep")), event);
}

pub fn publish_collateral_partial_released_event(
    env: &Env,
    event: CollateralPartialReleasedEvent,
) {
    env.events()
        .publish((symbol_short!("credit"), Symbol::new(env, "col_prel")), event);
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

/// Emit when `set_oracle_quorum_config` is called.
pub fn publish_oracle_quorum_config_set_event(
    env: &Env,
    min_quorum_k: u32,
    max_deviation_bps: u32,
    max_age_seconds: u64,
) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "orc_qcfg")),
        (min_quorum_k, max_deviation_bps, max_age_seconds),
    );
}

/// Emit when `submit_oracle_prices` successfully resolves a quorum price.
///
/// Data: `(resolved_price, min_quorum_k, timestamp)`.
pub fn publish_oracle_quorum_price_set_event(
    env: &Env,
    price: i128,
    quorum_k: u32,
    timestamp: u64,
) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "orc_qprc")),
        (price, quorum_k, timestamp),
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

/// Emitted when an admin forgives (writes off) a portion of a borrower's accrued interest.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebtForgivenEvent {
    /// Borrower whose debt was forgiven.
    pub borrower: Address,
    /// Amount of accrued interest written off.
    pub amount_forgiven: i128,
    /// Remaining accrued interest after the write-off.
    pub remaining_accrued_interest: i128,
    /// Outstanding utilized amount after the write-off.
    pub new_utilized_amount: i128,
}

/// Publish a debt forgiven event.
pub fn publish_debt_forgiven_event(env: &Env, event: DebtForgivenEvent) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "debt_frgv")),
        event,
    );
}

/// Structured borrow lifecycle event emitted at every state-changing borrow operation.
///
/// Complements the existing per-operation events (`drawn`, `repay`, `opened`, etc.)
/// with a single unified payload that captures the full credit-line snapshot at the
/// moment of the transition. Indexers can subscribe to `("credit", "borrow_lc")` to
/// reconstruct the complete lifecycle history without joining multiple event streams.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BorrowLifecycleEvent {
    /// Borrower address.
    pub borrower: Address,
    /// Lifecycle phase that triggered this event.
    pub phase: BorrowLifecyclePhase,
    /// Credit-line status after the operation.
    pub status: CreditStatus,
    /// Outstanding utilized amount after the operation.
    pub utilized_amount: i128,
    /// Credit limit at the time of the event.
    pub credit_limit: i128,
    /// Effective interest rate in basis points.
    pub interest_rate_bps: u32,
    /// Ledger timestamp of the event.
    pub timestamp: u64,
}

/// Discriminant for [`BorrowLifecycleEvent`] indicating which operation occurred.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BorrowLifecyclePhase {
    Opened,
    Drawn,
    Repaid,
    Suspended,
    Reinstated,
    Defaulted,
    Closed,
    DebtForgiven,
}

/// Publish a structured borrow lifecycle event.
///
/// Topic: `("credit", "borrow_lc")`.
pub fn publish_borrow_lifecycle_event(env: &Env, event: BorrowLifecycleEvent) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "borrow_lc")),
        event,
    );
}

/// Publish a grace waiver receipt event when a suspended line's accrual uses the grace period.
pub fn publish_grace_waiver_receipt_event(
    env: &Env,
    borrower: &Address,
    waived_amount: i128,
    mode: crate::types::GraceWaiverMode,
) {
    env.events().publish(
        (symbol_short!("credit"), symbol_short!("grace_wv")),
        GraceWaiverReceiptEvent {
            borrower: borrower.clone(),
            waived_amount,
            mode,
        },
    );
}



/// Emitted when a treasury withdrawal is proposed via `propose_treasury_withdrawal`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreasuryWithdrawalProposedEvent {
    /// Treasury recipient address.
    pub recipient: Address,
    /// Snapshot of the treasury balance at proposal time.
    pub amount: i128,
    /// Admin who submitted the proposal.
    pub proposer: Address,
    /// Ledger timestamp when the proposal was created.
    pub proposed_at: u64,
    /// Earliest timestamp at which execution is permitted (proposed_at + 86_400).
    pub execute_after: u64,
}

/// Emitted when a treasury withdrawal is executed via `execute_treasury_withdrawal`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreasuryWithdrawalExecutedEvent {
    /// Treasury recipient address.
    pub recipient: Address,
    /// Amount transferred.
    pub amount: i128,
    /// Admin who executed the withdrawal.
    pub executor: Address,
    /// Ledger timestamp at execution.
    pub executed_at: u64,
}

/// Publish a treasury withdrawal proposed event.
pub fn publish_treasury_withdrawal_proposed(env: &Env, event: TreasuryWithdrawalProposedEvent) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "tre_prop")),
        event,
    );
}

/// Publish a treasury withdrawal executed event.
pub fn publish_treasury_withdrawal_executed(env: &Env, event: TreasuryWithdrawalExecutedEvent) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "tre_exec")),
        event,
    );
}

/// Payload emitted when an admin commits a new attestation batch for a borrower.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttestationBatchCommittedEvent {
    /// Borrower whose attestation batch was updated.
    pub borrower: Address,
    /// SHA-256 Merkle root of all leaf hashes in the committed batch.
    pub merkle_root: soroban_sdk::BytesN<32>,
    /// Number of leaves in the batch (informational).
    pub count: u32,
}

/// Publish an attestation batch committed event.
pub fn publish_attestation_batch_committed(env: &Env, event: AttestationBatchCommittedEvent) {
    env.events().publish(
        (symbol_short!("credit"), Symbol::new(env, "atst_bat")),
        event,
    );
}
