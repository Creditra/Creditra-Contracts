// SPDX-License-Identifier: MIT
#![cfg_attr(not(test), no_std)]

//! Creditra freeze auth-boundary test crate (#835).
//!
//! Freeze controls live in [`creditra_credit::freeze`] and the matching
//! entrypoints on [`creditra_credit::Credit`]. This package anchors focused
//! per-entrypoint authorization boundary tests under `tests/auth_boundary.rs`.

pub use creditra_credit::*;
