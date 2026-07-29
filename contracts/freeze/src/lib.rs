// SPDX-License-Identifier: MIT
#![cfg_attr(not(test), no_std)]

//! Creditra freeze contract (v7).
//!
//! Freeze controls live in [`creditra_credit::freeze`] and the matching
//! entrypoints on [`creditra_credit::Credit`]. This package anchors focused
//! per-entrypoint authorization boundary tests and exposes a read-only
//! [`views::freeze_capabilities`] bitmap so clients can detect supported
//! freeze features at runtime.
//!
//! # Public surface
//!
//! | Module  | What                                                          |
//! |---------|---------------------------------------------------------------|
//! | `errors`| Stable ABI-pinned [`FreezeError`] catalog (mirror + specific) |
//! | `views` | Read-only [`freeze_capabilities`] bitmap (v7)                  |

pub use creditra_credit::*;

pub mod errors;
pub use errors::*;

pub mod views;
pub use views::*;
