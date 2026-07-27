// SPDX-License-Identifier: MIT

//! Structured lifecycle events for the freeze (v7) contract.
//!
//! # What
//!
//! Defines typed event structs and publisher helpers for every freeze
//! lifecycle transition. Off-chain indexers subscribe to these events
//! to track the full freeze/unfreeze history without polling storage.
//!
//! # Events
//!
//! | Event struct | Publisher fn | Topic | Trigger |
//! |---|---|---|---|
//! | [`DrawsFrozenEvent`] | [`publish_draws_frozen`] | `("freeze","drw_frz")` | `freeze_draws` / `unfreeze_draws` |
//! | [`CreditLineFrozenEvent`] | [`publish_credit_line_frozen`] | `("freeze","ln_frz")` | `freeze_credit_line` / `unfreeze_credit_line` |
//! | [`BorrowerFrozenEvent`] | [`publish_borrower_frozen`] | `("freeze","brw_frz")` | `freeze_borrower_until` |
//! | [`BorrowerUnfrozenEvent`] | [`publish_borrower_unfrozen`] | `("freeze","brw_ufz")` | `unfreeze_borrower` |
//!
//! # Topics
//!
//! All events are published under the `("freeze", _)` namespace using
//! `symbol_short!` (≤ 9 characters) for cheap on-chain encoding. Topics
//! are intentionally distinct from the `("credit", _)` namespace used by
//! the credit contract's internal events to allow independent subscriptions.
//!
//! # ABI Stability
//!
//! Event topics and payload field layouts form part of the public ABI.
//! Breaking changes require a new topic with a version suffix
//! (e.g., `("freeze","drw_frz2")`). Existing fields must never be removed
//! or reordered.
//!
//! # See also
//!
//! - `contracts/accrual/src/events.rs` — accrual event pattern.
//! - `contracts/credit/src/events.rs` — credit contract internal events.
//! - `docs/EVENTS_CATALOG.md` — canonical cross-contract event catalog.

use soroban_sdk::{contracttype, symbol_short, Address, Env};

use creditra_credit::FreezeReason;

// ── Event structs ─────────────────────────────────────────────────────────

/// Emitted when the global draws freeze state changes.
///
/// Published by both `freeze_draws` (with `frozen = true`) and
/// `unfreeze_draws` (with `frozen = false`). The `reason` field captures
/// the structured classification recorded at the time of the action.
///
/// # Topic
///
/// `("freeze", "drw_frz")`
///
/// # Fields
///
/// - `frozen` — `true` when draws are being frozen, `false` when unfreezing.
/// - `reason` — Structured classification of why the freeze was applied.
///   On `unfreeze_draws` this is the last stored reason before the freeze
///   was lifted.
/// - `timestamp` — Ledger timestamp at time of the action.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DrawsFrozenEvent {
    /// `true` = draws frozen, `false` = draws unfrozen.
    pub frozen: bool,
    /// Structured reason for the freeze/unfreeze action.
    pub reason: FreezeReason,
    /// Ledger timestamp at time of the action.
    pub timestamp: u64,
}

/// Emitted when a per-borrower credit line freeze state changes.
///
/// Published by both `freeze_credit_line` (with `frozen = true`) and
/// `unfreeze_credit_line` (with `frozen = false`).
///
/// # Topic
///
/// `("freeze", "ln_frz")`
///
/// # Fields
///
/// - `borrower` — The borrower whose credit line was frozen/unfrozen.
/// - `frozen` — `true` when the line is being frozen, `false` when unfreezing.
/// - `reason` — Structured classification recorded at freeze time. On
///   `unfreeze_credit_line` this is the reason that was in storage before
///   removal.
/// - `timestamp` — Ledger timestamp at time of the action.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditLineFrozenEvent {
    /// Borrower whose credit line was affected.
    pub borrower: Address,
    /// `true` = credit line frozen, `false` = credit line unfrozen.
    pub frozen: bool,
    /// Structured reason for the freeze/unfreeze action.
    pub reason: FreezeReason,
    /// Ledger timestamp at time of the action.
    pub timestamp: u64,
}

/// Emitted when a borrower is placed under a time-bounded freeze.
///
/// Published by `freeze_borrower_until`. The freeze automatically expires
/// once `env.ledger().timestamp() >= frozen_until`.
///
/// # Topic
///
/// `("freeze", "brw_frz")`
///
/// # Fields
///
/// - `borrower` — The borrower being frozen.
/// - `frozen_until` — Ledger timestamp at which the freeze expires.
/// - `timestamp` — Ledger timestamp at time of the action.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BorrowerFrozenEvent {
    /// Borrower being placed under a time-bounded freeze.
    pub borrower: Address,
    /// Ledger timestamp at which the freeze will expire.
    pub frozen_until: u64,
    /// Ledger timestamp at time of the action.
    pub timestamp: u64,
}

/// Emitted when a borrower freeze is explicitly lifted before expiry.
///
/// Published by `unfreeze_borrower`. Not emitted when a freeze expires
/// naturally via timestamp — callers must check `is_borrower_frozen`
/// to determine active state.
///
/// # Topic
///
/// `("freeze", "brw_ufz")`
///
/// # Fields
///
/// - `borrower` — The borrower being unfrozen.
/// - `timestamp` — Ledger timestamp at time of the action.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BorrowerUnfrozenEvent {
    /// Borrower whose freeze was explicitly lifted.
    pub borrower: Address,
    /// Ledger timestamp at time of the action.
    pub timestamp: u64,
}

// ── Publisher functions ────────────────────────────────────────────────────

/// Publish a [`DrawsFrozenEvent`] to the Soroban event ledger.
///
/// # Parameters
///
/// * `env` — The Soroban environment reference.
/// * `frozen` — `true` when draws are being frozen; `false` when unfreezing.
/// * `reason` — Structured [`FreezeReason`] classification for this action.
///
/// # Topic
///
/// `("freeze", "drw_frz")`
///
/// # Example
///
/// ```ignore
/// publish_draws_frozen(&env, true, FreezeReason::LiquidityReserve);
/// ```
pub fn publish_draws_frozen(env: &Env, frozen: bool, reason: FreezeReason) {
    env.events().publish(
        (symbol_short!("freeze"), symbol_short!("drw_frz")),
        DrawsFrozenEvent {
            frozen,
            reason,
            timestamp: env.ledger().timestamp(),
        },
    );
}

/// Publish a [`CreditLineFrozenEvent`] to the Soroban event ledger.
///
/// # Parameters
///
/// * `env` — The Soroban environment reference.
/// * `borrower` — The borrower whose credit line was affected.
/// * `frozen` — `true` when the line is being frozen; `false` when unfreezing.
/// * `reason` — Structured [`FreezeReason`] classification for this action.
///
/// # Topic
///
/// `("freeze", "ln_frz")`
///
/// # Example
///
/// ```ignore
/// publish_credit_line_frozen(&env, &borrower, true, FreezeReason::Compliance);
/// ```
pub fn publish_credit_line_frozen(
    env: &Env,
    borrower: &Address,
    frozen: bool,
    reason: FreezeReason,
) {
    env.events().publish(
        (symbol_short!("freeze"), symbol_short!("ln_frz")),
        CreditLineFrozenEvent {
            borrower: borrower.clone(),
            frozen,
            reason,
            timestamp: env.ledger().timestamp(),
        },
    );
}

/// Publish a [`BorrowerFrozenEvent`] to the Soroban event ledger.
///
/// # Parameters
///
/// * `env` — The Soroban environment reference.
/// * `borrower` — The borrower being placed under a time-bounded freeze.
/// * `frozen_until` — Ledger timestamp at which the freeze expires.
///
/// # Topic
///
/// `("freeze", "brw_frz")`
///
/// # Example
///
/// ```ignore
/// publish_borrower_frozen(&env, &borrower, expiry_ts);
/// ```
pub fn publish_borrower_frozen(env: &Env, borrower: &Address, frozen_until: u64) {
    env.events().publish(
        (symbol_short!("freeze"), symbol_short!("brw_frz")),
        BorrowerFrozenEvent {
            borrower: borrower.clone(),
            frozen_until,
            timestamp: env.ledger().timestamp(),
        },
    );
}

/// Publish a [`BorrowerUnfrozenEvent`] to the Soroban event ledger.
///
/// # Parameters
///
/// * `env` — The Soroban environment reference.
/// * `borrower` — The borrower whose freeze was explicitly lifted.
///
/// # Topic
///
/// `("freeze", "brw_ufz")`
///
/// # Example
///
/// ```ignore
/// publish_borrower_unfrozen(&env, &borrower);
/// ```
pub fn publish_borrower_unfrozen(env: &Env, borrower: &Address) {
    env.events().publish(
        (symbol_short!("freeze"), symbol_short!("brw_ufz")),
        BorrowerUnfrozenEvent {
            borrower: borrower.clone(),
            timestamp: env.ledger().timestamp(),
        },
    );
}
