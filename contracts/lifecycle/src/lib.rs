// SPDX-License-Identifier: MIT
#![cfg_attr(not(test), no_std)]

//! # Creditra lifecycle v7 — credit-line state machine
//!
//! Thin crate that re-exports the full [`creditra_credit`] surface for
//! error-stability testing, gas-snapshot regression guards, and compositional
//! reuse of the v7 lifecycle engine. The lifecycle engine itself lives in
//! [`creditra_credit::lifecycle`] and the read-only capabilities view in
//! [`creditra_credit::lifecycle_views`].
//!
//! ---
//!
//! ## What
//!
//! Every state-changing lifecycle entrypoint implements a transition in the
//! credit-line state machine (`Active`, `Suspended`, `Defaulted`, `Closed`,
//! `Restricted`). Each transition gates on the protocol pause flag, applies
//! interest accrual before mutating state, enforces monotonic timestamps,
//! persists via [`crate::storage::persist_credit_line`], and emits a
//! [`crate::events::CreditLineEvent`] on the appropriate `("credit", _)` topic.
//!
//! The lifecycle surface includes:
//!
//! - **Origination** — `open_credit_line` creates or admin-re-opens a line.
//! - **Suspension** — `suspend_credit_line` (admin) and
//!   `self_suspend_credit_line` (borrower safety control).
//! - **Closure** — `close_credit_line` (admin force-close or borrower
//!   self-close when zero utilization), `close_credit_lines_batch`.
//! - **Default & cure** — `default_credit_line` (Active/Restricted/Suspended
//!   → Defaulted), `reinstate_credit_line` (Defaulted → Active or
//!   Restricted).
//! - **Settlement** — `settle_default_liquidation` (admin-only,
//!   replay-protected cross-contract handoff with the auction),
//!   `forgive_debt` (accounting-only write-off).
//! - **Configuration** — `set_credit_limit_bounds`,
//!   `set_per_borrower_liquidation_grace`, `set_repayment_schedule`,
//!   `set_late_fee_flat`, `set_late_fee_config`.
//! - **Admin cooldown** — `set_admin_lifecycle_cooldown_seconds` configures
//!   a minimum interval between critical lifecycle admin actions.
//! - **Read-only views** — `lifecycle_capabilities` (pre-flight bitmap for
//!   every state-changing lifecycle operation), `get_credit_line`,
//!   `get_credit_limit_bounds`, `get_per_borrower_liquidation_grace`,
//!   `get_repayment_schedule`, `get_late_fee_flat`, `get_late_fee_config`.
//!
//! ---
//!
//! ## State-transition entrypoints
//!
//! ### `open_credit_line`
//!
//! ```ignore
//! fn open_credit_line(
//!     env: Env,
//!     borrower: Address,
//!     credit_limit: i128,
//!     interest_rate_bps: u32,
//!     risk_score: u32,
//! )
//! ```
//!
//! # Authorization
//! Admin only via [`require_admin_auth`]. Gated by `enforce_borrow_admin_cooldown`.
//!
//! # Parameters
//! - `borrower` — Address of the borrower.
//! - `credit_limit` — Must be positive and within `[MinCreditLimit, MaxCreditLimit]`
//!   bounds if configured. Reverts with [`ContractError::InvalidAmount`] or
//!   [`ContractError::LimitOutOfBounds`].
//! - `interest_rate_bps` — ≤ 10 000 bps (100 %). Reverts [`ContractError::RateTooHigh`].
//! - `risk_score` — ≤ `MAX_RISK_SCORE`. Reverts [`ContractError::ScoreTooHigh`].
//!
//! # Behaviour
//! - **New line**: Creates an `Active` line with `utilized_amount = 0`.
//! - **Existing Active line**: Reverts [`ContractError::AlreadyInitialized`] (14).
//! - **Existing non-Active line**: Admin re-open is permitted; requires admin auth.
//! - Resets the repayment schedule if one existed.
//!
//! # Events
//! Emits `("credit", "opened")` [`CreditLineEvent`].
//!
//! # Storage
//! Writes to persistent storage (per-borrower `CreditLineData`). Updates
//! `TotalUtilized` accumulator.
//!
//! ### `suspend_credit_line`
//!
//! ```ignore
//! fn suspend_credit_line(env: Env, borrower: Address)
//! ```
//!
//! # Authorization
//! Admin only via the `lib.rs` entrypoint wrapper (not re-checked in the
//! lifecycle module to avoid double `require_auth` in the same Soroban
//! invocation frame).
//!
//! # State transition
//! `Active → Suspended`
//!
//! # Panics
//! - [`ContractError::CreditLineNotFound`] (3) — no credit line exists.
//! - [`ContractError::CreditLineSuspended`] (20) — line is not `Active`.
//!
//! # Events
//! Emits `("credit", "suspend")` [`CreditLineEvent`].
//!
//! # Storage
//! Sets `suspension_ts` to the current ledger timestamp. Bumps persistent
//! TTL on the credit-line entry.
//!
//! ### `self_suspend_credit_line`
//!
//! ```ignore
//! fn self_suspend_credit_line(env: Env, borrower: Address)
//! ```
//!
//! # Authorization
//! Borrower must authorize (`borrower.require_auth()`). No admin involvement.
//!
//! # State transition
//! `Active → Suspended`
//!
//! # Behaviour
//! Borrower safety control: blocks future draws while leaving repayments
//! available. Reactivation requires a separate admin workflow.
//!
//! # Events
//! Same internal path as `suspend_credit_line` — emits `("credit", "suspend")`.
//!
//! # Storage
//! Loads via [`crate::storage::get_credit_line`] which bumps persistent TTL
//! on read.
//!
//! ### `close_credit_line`
//!
//! ```ignore
//! fn close_credit_line(env: Env, borrower: Address, closer: Address)
//! ```
//!
//! # Authorization
//! - **Admin closer**: Always permitted (force-close from any non-Closed status).
//! - **Borrower closer**: Permitted only when `utilized_amount == 0`.
//! - **Third party**: Reverts [`ContractError::Unauthorized`] (1).
//!
//! # State transition
//! Any non-Closed status → `Closed`.
//!
//! # Idempotency
//! Already-Closed lines return immediately without error or event emission.
//!
//! # Panics
//! - [`ContractError::CreditLineNotFound`] (3).
//! - [`ContractError::UtilizationNotZero`] (10) — borrower close with
//!   outstanding balance.
//!
//! # Events
//! Emits `("credit", "closed")` [`CreditLineEvent`] on successful state
//! change (not on idempotent re-close). Clears the repayment schedule.
//!
//! ### `close_credit_lines_batch`
//!
//! ```ignore
//! fn close_credit_lines_batch(env: Env, borrowers: Vec<Address>)
//! ```
//!
//! # Authorization
//! Admin only. Resolves admin once to amortise the storage read.
//!
//! # Atomicity
//! Reverts on the first failure — the entire batch is all-or-nothing.
//!
//! # Parameters
//! - `borrowers` — List of borrower addresses. Length must be ≤ `BATCH_CLOSE_MAX` (50).
//!
//! ### `default_credit_line`
//!
//! ```ignore
//! fn default_credit_line(env: Env, borrower: Address)
//! ```
//!
//! # Authorization
//! Admin only.
//!
//! # State transition
//! `Active` | `Restricted` | `Suspended` → `Defaulted`.
//!
//! # Behaviour
//! - Closed lines revert [`ContractError::CreditLineClosed`] (4).
//! - Already-Defaulted lines return idempotently.
//! - Respects the per-borrower liquidation grace period when configured.
//!
//! # Events
//! Emits `("credit", "defaulted")` [`CreditLineEvent`] and
//! `("credit", "liq_req")` for the off-chain liquidation orchestrator.
//!
//! # Storage
//! Loads via [`crate::storage::get_credit_line`] which bumps persistent TTL.
//!
//! ### `reinstate_credit_line`
//!
//! ```ignore
//! fn reinstate_credit_line(env: Env, borrower: Address, target_status: CreditStatus)
//! ```
//!
//! # Authorization
//! Admin only.
//!
//! # State transition
//! `Defaulted → Active` or `Defaulted → Restricted`.
//!
//! # Parameters
//! - `target_status` — Must be `Active` or `Restricted`. Other values revert
//!   [`ContractError::InvalidAmount`] (5).
//!
//! # Panics
//! - [`ContractError::CreditLineNotFound`] (3).
//! - [`ContractError::CreditLineDefaulted`] (21) — current status is not
//!   `Defaulted`.
//!
//! # Events
//! Emits `("credit", "reinstate")` [`CreditLineEvent`]. Resets
//! `suspension_ts` to `0`.
//!
//! # Storage
//! Bumps persistent TTL on read.
//!
//! ### `forgive_debt`
//!
//! ```ignore
//! fn forgive_debt(env: Env, borrower: Address, amount: i128)
//! ```
//!
//! # Authorization
//! Admin only.
//!
//! # Behaviour
//! Accounting-only write-off: reduces `accrued_interest` first, then
//! `utilized_amount`, by `amount` (clamped to outstanding balance). No token
//! transfer occurs.
//!
//! # Panics
//! - [`ContractError::InvalidAmount`] (5) — `amount <= 0`.
//! - [`ContractError::CreditLineNotFound`] (3).
//!
//! # Events
//! Emits `DebtForgivenEvent` and `BorrowLifecycleEvent { phase: DebtForgiven }`.
//!
//! ### `settle_default_liquidation`
//!
//! ```ignore
//! fn settle_default_liquidation(
//!     env: Env,
//!     borrower: Address,
//!     recovered_amount: i128,
//!     settlement_id: Symbol,
//!     close_factor_bps: u32,
//! )
//! ```
//!
//! # Authorization
//! Admin only.
//!
//! # Behaviour
//! Applies auction liquidation proceeds to a Defaulted line. Reduces
//! `utilized_amount` by `actual_recovery` (capped by
//! `utilized_amount * close_factor_bps / 10_000`). If `utilized_amount`
//! reaches `0`, transitions to `Closed`.
//!
//! # Replay protection
//! The `(borrower, settlement_id)` pair is persisted; duplicate settlement
//! reverts [`ContractError::AlreadyInitialized`] (14).
//!
//! # Parameters
//! - `recovered_amount` — Must be positive and ≤ `max_recoverable`.
//! - `settlement_id` — Unique per-settlement identifier (replay protection).
//! - `close_factor_bps` — Must be in `(0, 10_000]` and ≤ the protocol-level
//!   `close_factor_bps` cap.
//!
//! # Events
//! Emits `DefaultLiquidationSettledEvent`. If fully settled, also emits
//! `("credit", "closed")` [`CreditLineEvent`].
//!
//! ---
//!
//! ## Configuration entrypoints
//!
//! ### `set_credit_limit_bounds`
//!
//! ```ignore
//! fn set_credit_limit_bounds(env: Env, min: i128, max: i128)
//! ```
//!
//! Admin only. Sets the global `[MinCreditLimit, MaxCreditLimit]` bounds
//! enforced on `open_credit_line` and `update_risk_parameters`. `min >= 0`,
//! `max >= min`. Reverts [`ContractError::InvalidAmount`] (5) or
//! [`ContractError::LimitOutOfBounds`] (34). Writes to instance storage.
//!
//! ### `get_credit_limit_bounds`
//!
//! ```ignore
//! fn get_credit_limit_bounds(env: Env) -> (Option<i128>, Option<i128>)
//! ```
//!
//! Returns the configured `(min, max)` bounds. Each is `None` when unset.
//! No auth required.
//!
//! ### `set_per_borrower_liquidation_grace`
//!
//! ```ignore
//! fn set_per_borrower_liquidation_grace(env: Env, borrower: Address, grace_period_seconds: u64)
//! ```
//!
//! Admin only. Sets a per-borrower grace period during which
//! `default_credit_line` is blocked, measured from `suspension_ts` (or
//! the next due / last rate update / last accrual timestamp when
//! `suspension_ts` is `0`). Pass `0` to remove.
//!
//! # Panics
//! - [`ContractError::CreditLineNotFound`] (3).
//! - [`ContractError::CreditLineClosed`] (4).
//!
//! # Storage
//! Writes to persistent storage under `DataKey::PerBorrowerLiquidationGrace`.
//!
//! ### `get_per_borrower_liquidation_grace`
//!
//! ```ignore
//! fn get_per_borrower_liquidation_grace(env: Env, borrower: Address) -> u64
//! ```
//!
//! Returns the configured per-borrower liquidation grace period in seconds.
//! Returns `0` when unset. No auth required. Read-only persistent storage.
//!
//! ### `set_repayment_schedule`
//!
//! ```ignore
//! fn set_repayment_schedule(
//!     env: Env,
//!     borrower: Address,
//!     amount_per_period: i128,
//!     period_seconds: u64,
//!     first_due_ts: u64,
//! )
//! ```
//!
//! Admin only. Configures an installment repayment schedule for `borrower`.
//! `amount_per_period` and `period_seconds` must be positive. Reverts
//! [`ContractError::InvalidAmount`] (5) or
//! [`ContractError::CreditLineNotFound`] (3). Writes to persistent storage
//! and bumps the credit-line TTL.
//!
//! ### `get_repayment_schedule`
//!
//! ```ignore
//! fn get_repayment_schedule(env: Env, borrower: Address) -> Option<RepaymentSchedule>
//! ```
//!
//! Returns the configured `RepaymentSchedule` or `None`. No auth required.
//!
//! ### `set_late_fee_flat`
//!
//! ```ignore
//! fn set_late_fee_flat(env: Env, fee: i128)
//! ```
//!
//! Admin only. Sets a flat late fee charged per overdue installment to
//! `TreasuryBalance`. `fee >= 0`; negative reverts
//! [`ContractError::InvalidAmount`] (5). Writes to instance storage.
//!
//! ### `get_late_fee_flat`
//!
//! ```ignore
//! fn get_late_fee_flat(env: Env) -> i128
//! ```
//!
//! Returns the configured flat late fee. Returns `0` when unset. No auth required.
//!
//! ### `set_late_fee_config`
//!
//! ```ignore
//! fn set_late_fee_config(env: Env, config: Option<LateFeeConfig>)
//! ```
//!
//! Admin only. Structured late-fee configuration (`Flat` or `AprBased`).
//! `Flat` mode: `amount >= 0`. `AprBased` mode: `surcharge_bps <= 10_000`.
//! Pass `None` to remove and fall back to legacy keys. Writes to instance
//! storage via `DataKey::LateFeeConfig`.
//!
//! ### `get_late_fee_config`
//!
//! ```ignore
//! fn get_late_fee_config(env: Env) -> Option<LateFeeConfig>
//! ```
//!
//! Returns the structured late-fee configuration. `None` means legacy keys
//! are in use. No auth required.
//!
//! ---
//!
//! ## Admin cooldown entrypoints
//!
//! ### `set_admin_lifecycle_cooldown_seconds`
//!
//! ```ignore
//! fn set_admin_lifecycle_cooldown_seconds(env: Env, seconds: u64)
//! ```
//!
//! Admin only. Configures the minimum interval between critical lifecycle
//! admin actions (`set_credit_limit_bounds`,
//! `set_per_borrower_liquidation_grace`, `set_repayment_schedule`,
//! `set_late_fee_flat`, `set_late_fee_config`). Pass `0` to disable.
//! Gated by `assert_not_paused`.
//!
//! # Storage
//! Writes to instance storage under `DataKey::AdminLifecycleCooldownSeconds`.
//!
//! ### `get_admin_lifecycle_cooldown_seconds`
//!
//! ```ignore
//! fn get_admin_lifecycle_cooldown_seconds(env: Env) -> Option<u64>
//! ```
//!
//! Returns the configured admin lifecycle cool-off interval. `None` when
//! never configured (equivalent to disabled). No auth required.
//!
//! ### `get_last_admin_lifecycle_critical_action_ts`
//!
//! ```ignore
//! fn get_last_admin_lifecycle_critical_action_ts(env: Env) -> Option<u64>
//! ```
//!
//! Returns the ledger timestamp of the last critical lifecycle admin action.
//! `None` when no critical action has been performed. No auth required.
//!
//! ---
//!
//! ## Read-only views
//!
//! ### `lifecycle_capabilities`
//!
//! ```ignore
//! fn lifecycle_capabilities(env: Env, borrower: Address) -> LifecycleCapabilities
//! ```
//!
//! Read-only, no-auth pre-flight check for every state-changing lifecycle
//! entrypoint. Returns a [`LifecycleCapabilities`] bitmap with six bool
//! fields: `can_suspend`, `can_self_suspend`, `can_close_admin`,
//! `can_close_borrower`, `can_default`, `can_reinstate`. All fields are
//! `false` when no credit line exists or when the protocol is paused.
//!
//! # Returns
//! See [`LifecycleCapabilities`] for field-level semantics.
//!
//! ### `get_credit_line`
//!
//! ```ignore
//! fn get_credit_line(env: Env, borrower: Address) -> Option<CreditLineData>
//! ```
//!
//! Returns the full `CreditLineData` for `borrower`, or `None` if no line
//! exists. No auth required. Hot reads bump persistent TTL on the
//! credit-line entry when the remaining lifetime falls below the configured
//! threshold.
//!
//! ---
//!
//! ## How
//!
//! - **Storage tiers.** Hot configuration in Instance storage (credit-limit
//!   bounds, admin cooldown config, late-fee config); per-borrower state in
//!   Persistent storage with TTL auto-bumped on every access. See
//!   [`crate::storage`].
//! - **Accrual.** Every transition calls
//!   [`crate::accrual::apply_accrual`] before reading `utilized_amount`, so
//!   the transition acts on capitalized debt.
//! - **Timestamp monotonicity.** Every timestamp write (`suspension_ts`,
//!   `last_rate_update_ts`) is gated by
//!   [`crate::storage::assert_ts_monotonic`]; backward writes revert
//!   [`ContractError::TimestampRegression`] (33).
//! - **Atomic persistence.** Every transition calls
//!   [`crate::storage::persist_credit_line`] with the captured
//!   `previous_utilized` so the global `TotalUtilized` accumulator stays
//!   consistent.
//! - **Replay safety.** Settlement uses `(borrower, settlement_id)` as the
//!   dedup key, stored in persistent storage.
//!
//! ## State machine
//!
//! ```text
//!                  open_credit_line
//!                       │
//!                       ▼
//!              ┌───────────────┐
//!              │    Active     │◄────────── reinstate_credit_line ────┐
//!              └───┬───┬───┬───┘                                       │
//!                  │   │   │                                           │
//!     suspend /    │   │   │   close_credit_line                       │
//!     self_suspend │   │   └──────────────┐                            │
//!                  │   │                  │                            │
//!                  ▼   │  default_        ▼                            │
//!     ┌──────────────┐│  credit_line  ┌──────────┐                     │
//!     │  Suspended   ├┘               │  Closed  │   (terminal)        │
//!     └──────┬───────┘       ┌───────►└──────────┘                     │
//!            │               │                                         │
//!            │  default_     │                                         │
//!            │  credit_line  │                                         │
//!            │               │                                         │
//!            ▼               │                                         │
//!     ┌──────────────┐       │                                         │
//!     │  Defaulted   ├───────┘                                         │
//!     └──────┬───────┘                                                 │
//!            │                                                         │
//!            │  settle_default_liquidation (partial)                   │
//!            │  ── status stays Defaulted                              │
//!            │                                                         │
//!            │  settle_default_liquidation (full) or close_credit_line │
//!            │  ── status becomes Closed                               │
//!            │                                                         │
//!            └──────────────── reinstate_credit_line ──────────────────┘
//!
//!   Restricted is a repayment-capable cure state created by
//!   `update_risk_parameters` when a limit decrease drops the configured
//!   limit below current utilization. Repayments auto-cure back to Active
//!   when `utilized_amount <= credit_limit`.
//! ```
//!
//! ## Security invariants
//!
//! - `TotalUtilized == Σ utilized_amount` over open lines.
//! - Every state transition gates on `assert_not_paused`.
//! - Admin-only entrypoints call `require_admin_auth`.
//! - Borrower-path entrypoints (`self_suspend_credit_line`,
//!   `close_credit_line` with `closer == borrower`) call
//!   `borrower.require_auth()`.
//! - Monotonic timestamps; backward writes revert
//!   `ContractError::TimestampRegression = 33`.
//! - Settlement is replay-protected via `(borrower, settlement_id)`.
//! - `CloseFactorBps` caps the maximum recoverable amount in
//!   `settle_default_liquidation`.
//! - 35+ `ContractError` discriminants are ABI-stable; CI test
//!   [`tests/err_stab.rs`] reverts on reorder.
//!
//! ## See also
//!
//! - [`contracts/credit/src/lifecycle.rs`] — the v7 lifecycle engine.
//! - [`contracts/credit/src/lib.rs`] — the Credit contract entrypoints.
//! - [`docs/state-machine.md`](../../../docs/state-machine.md) — the
//!   authoritative transition table.
//! - [`docs/default-liquidation-auction-hook.md`](../../../docs/default-liquidation-auction-hook.md)
//!   — the cross-contract settlement handoff protocol.
//! - [`tests/err_stab.rs`] — error discriminant stability pins.
//! - [`tests/gas_snap.rs`] — per-entrypoint gas snapshots.
//! - [`tests/admin_cooldown.rs`] — admin lifecycle cooldown regression tests.
//! - [`tests/capabilities.rs`] — `lifecycle_capabilities` view coverage.

pub use creditra_credit::*;
