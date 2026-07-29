// SPDX-License-Identifier: MIT
#![cfg_attr(not(test), no_std)]

//! # Creditra Borrow Crate
//!
//! Error-stability, authorization-snapshot, and gas-snapshot test anchor for
//! the **borrow / draw / repay** subsystem of the Creditra protocol.
//!
//! ## Overview
//!
//! This crate does **not** define any entrypoints itself. Instead it
//! re-exports the full public surface of [`creditra_credit`], which is the
//! canonical [`Credit`] Soroban smart contract. Its primary purpose is to
//! host focused CI test suites that pin client-facing
//! [`ContractError`](crate::types::ContractError) discriminants, authorization
//! checks, and gas profiles for the v7 borrow subsystem (see issue #847,
//! #804). See [`tests/err_stab.rs`], [`tests/auth_snap.rs`], and
//! [`tests/gas_snap.rs`] for the respective test harnesses.
//!
//! ## Re-exported Borrow Entrypoints
//!
//! The following public entrypoints (among others re-exported from
//! [`creditra_credit`]) are the primary borrow API surface:
//!
//! | Entrypoint | Description | Auth |
//! |---|---|--|
//! | [`draw_credit`](Credit::draw_credit) | Draw liquidity tokens up to the available credit limit | `borrower.require_auth()` |
//! | [`repay_credit`](Credit::repay_credit) | Repay drawn principal plus accrued interest | `borrower.require_auth()` |
//! | [`open_credit_line`](Credit::open_credit_line) | Open a new credit line for a borrower | `admin.require_auth()` |
//! | [`update_risk_parameters`](Credit::update_risk_parameters) | Update a borrower's risk score and/or interest rate | varies |
//! | [`close_credit_line`](Credit::close_credit_line) | Close an active credit line | `admin.require_auth()` (or `borrower` self-close) |
//! | [`default_credit_line`](Credit::default_credit_line) | Mark a credit line as defaulted | keeper (anyone) |
//! | [`suspend_credit_line`](Credit::suspend_credit_line) | Suspend draws on a credit line | `admin.require_auth()` |
//! | [`reinstate_credit_line`](Credit::reinstate_credit_line) | Reinstate a suspended or restricted line | `admin.require_auth()` |
//! | [`reverse_draw`](Credit::reverse_draw) | Reverse an erroneous draw within a time window | `admin.require_auth()` |
//!
//! ## Authorization
//!
//! - **Draw / Repay:** The borrower must authorize via `require_auth()`.
//! - **Admin operations** (open, close, suspend, reinstate, risk changes):
//!   The configured contract admin must authorize.
//! - **Default:** Permissionless (anyone can trigger a default when the
//!   health factor drops below the threshold).
//! - **Reversal:** Admin-only, within a 1-hour window.
//!
//! ## Errors
//!
//! All error discriminants are documented in
//! [`ContractError`](crate::types::ContractError). Key borrow errors include:
//!
//! | Discriminant | Meaning |
//! |---|---|
//! | `#1` - `InvalidAmount` | Amount is zero or negative |
//! | `#2` - `CreditLineNotFound` | No credit line exists for the borrower |
//! | `#3` - `CreditLineSuspended` | Line is in `Suspended` status |
//! | `#4` - `CreditLineDefaulted` | Line is in `Defaulted` status |
//! | `#5` - `CreditLineClosed` | Line is in `Closed` status |
//! | `#7` - `InsufficientCreditLimit` | Draw would exceed the available limit |
//! | `#9` - `RepayExceedsUtilized` | Repay amount exceeds drawn + accrued |
//! | `#11` - `Reentrancy` | Reentrant call detected via the instance guard |
//! | `#12` - `Overflow` | Arithmetic overflow in `i128` accounting |
//! | `#22` - `LiquidityTokenNotConfigured` | `set_liquidity_token` not yet called |
//! | `#23` - `LiquiditySourceNotConfigured` | `set_liquidity_source` not yet called |
//! | `#24` - `InsufficientReserve` | Contract reserve has insufficient liquidity |
//!
//! ## Security Considerations
//!
//! - **Reentrancy guard:** Both [`draw_credit`](Credit::draw_credit) and
//!   [`repay_credit`](Credit::repay_credit) set an instance-level reentrancy
//!   guard before performing any token CPI. The guard is cleared on every
//!   exit path (success and failure).
//! - **Overflow-safe arithmetic:** All `i128` operations use `checked_*`
//!   primitives; the release profile enables `overflow-checks`. An overflow
//!   reverts with `ContractError::Overflow`.
//! - **Accrual-before-mutation:** Every borrow mutation calls
//!   `apply_accrual` to realize outstanding interest before altering
//!   `utilized_amount` or `credit_limit`.
//! - **Status gating:** Draws are blocked on `Suspended`, `Defaulted`, and
//!   `Closed` lines. Repayments are never pause-gated — borrowers must
//!   always be able to deleverage.
//! - **Oracle circuit breaker:** [`draw_credit`](Credit::draw_credit)
//!   verifies that the reported oracle price deviation does not exceed the
//!   configured threshold when collateral is present.
//! - **Draw cooldown:** An optional per-borrower minimum interval between
//!   successive draws can be configured via
//!   [`set_draw_min_interval`](Credit::set_draw_min_interval).
//! - **Max draw / repay caps:** Configurable per-protocol limits via
//!   [`set_max_draw_amount`](Credit::set_max_draw_amount) and
//!   [`set_max_repay_amount`](Credit::set_max_repay_amount).
//!
//! ## Test Suites
//!
//! - **`err_stab`** — Freezes client-facing [`ContractError`] numeric
//!   discriminants; CI reverts if any discriminant is reordered or removed.
//! - **`auth_snap`** — Snapshots authorization requirements for each
//!   entrypoint; CI catches unintended auth changes.
//! - **`gas_snap`** — Records per-entrypoint CPU and memory costs for
//!   gas-regression baselines.
//!
//! ## Related Crates
//!
//! - [`creditra_credit`] — The canonical smart contract with all entrypoints.
//! - `creditra_collateral` — Collateral deposit / withdrawal subsystem.
//! - `creditra_lifecycle` — Lifecycle management (close, suspend, default,
//!   reinstate, forgive).
//! - `creditra_accrual` — Interest accrual engine.
//! - `creditra_risk` — Risk scoring and rate calculation.
//! - `creditra_query` — Read-only query views.

/// Re-export of the full [`creditra_credit`] public API.
///
/// This includes the [`Credit`] contract type, its associated
/// [`CreditClient`](crate::CreditClient), the
/// [`ContractError`](crate::types::ContractError) enum,
/// [`CreditLineData`](crate::types::CreditLineData), all helper types in
/// [`crate::types`], and every other public symbol from the canonical
/// Creditra credit contract.
///
/// # Usage
///
/// ```ignore
/// use creditra_borrow::CreditClient;
/// use creditra_borrow::types::ContractError;
/// ```
pub use creditra_credit::*;
