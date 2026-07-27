// SPDX-License-Identifier: MIT
#![cfg_attr(not(test), no_std)]

//! Creditra freeze contract (v7).
//!
//! Freeze controls live in [`creditra_credit::freeze`] and the matching
//! entrypoints on [`creditra_credit::Credit`]. This crate:
//!
//! - Re-exports the full `creditra_credit` surface so test harnesses only
//!   need one dependency.
//! - Provides [`events`] — structured lifecycle event types and publisher
//!   helpers for every freeze transition, for use by off-chain indexers.
//! - Anchors per-entrypoint authorization boundary tests under
//!   `tests/auth_boundary.rs`.
//!
//! # Events
//!
//! | Event | Topic | Trigger |
//! |---|---|---|
//! | [`events::DrawsFrozenEvent`] | `("freeze","drw_frz")` | `freeze_draws` / `unfreeze_draws` |
//! | [`events::CreditLineFrozenEvent`] | `("freeze","ln_frz")` | `freeze_credit_line` / `unfreeze_credit_line` |
//! | [`events::BorrowerFrozenEvent`] | `("freeze","brw_frz")` | `freeze_borrower_until` |
//! | [`events::BorrowerUnfrozenEvent`] | `("freeze","brw_ufz")` | `unfreeze_borrower` |

pub mod events;

pub use creditra_credit::*;
