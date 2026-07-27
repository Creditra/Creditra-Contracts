// SPDX-License-Identifier: MIT

//! `creditra-collateral` — stable ContractError catalog for collateral operations.
//!
//! # Purpose
//!
//! This crate publishes a stable, scoped [`CollateralError`] catalog for the
//! Creditra collateral domain. The catalog is the **source of truth** for the
//! integer error codes emitted by current and future contract paths that
//! handle collateral deposits, withdrawals, ratio checks, and the
//! admin-managed collateral allowlist.
//!
//! # Stability
//!
//! The discriminants exported here are **permanent on deployment** for the
//! Creditra collateral contract wasm. Once this catalog is published, the
//! following invariants apply (mirroring the conventions enforced by
//! `contracts/credit/tests/error_discriminants.rs`):
//!
//! - Existing variants must **never** be reordered or renumbered.
//! - New variants must always be **appended** with the next available integer.
//! - Adding/removing a variant requires updating the integration test
//!   (`tests/catalog.rs`) and `docs/errors/collateral.md` in the same change.
//!
//! # Two-tier discriminant policy
//!
//! The catalog contains two tiers of variants:
//!
//! 1. **Mirror tier** (codes `5`, `12`, `22`, `35`, `39`):
//!    Variants that match the canonical `ContractError` codes published by
//!    `contracts/credit/src/types.rs`. The collateral domain reuses these
//!    errors verbatim because they convey the same semantic meaning
//!    (e.g. *"withdrawal amount exceeds deposited balance"*). SDK consumers
//!    can map these codes against the canonical table at
//!    [`docs/ERROR_CODES.md`](../../../docs/ERROR_CODES.md).
//!
//! 2. **Collateral-specific tier** (codes `100+`):
//!    New variants exclusive to the collateral contract. These occupy the
//!    `100+` namespace deliberately — the credit contract uses `1..49` and
//!    the gap ensures no visual collision if a future PR appends to either
//!    catalog.
//!
//! # Why a separate crate?
//!
//! - **ABI isolation**: when this catalog is wired into a deployed
//!   collateral contract, its discriminants form their own ABI namespace;
//!   SDK consumers decode the discriminant against the
//!   *contract they invoked*, not against the global table.
//! - **Review hygiene**: changes to this catalog cannot accidentally
//!   destabilise `contracts/credit/tests/error_discriminants.rs` — the
//!   canonical credit test is untouched.
//! - **Forward compatibility**: when the collateral contract logic lands,
//!   it can adopt this enum verbatim without re-deriving any discriminant.
//!
//! # Security
//!
//! - No `unwrap()` calls are present in the catalog data path (the enum is
//!   pure value-type data).
//! - The `#[contracterror]` derive enforces `#[repr(u32)]`, which is the
//!   Soroban host boundary for contract-emitted errors.
//!
//! [`CollateralError`]: errors::CollateralError


pub mod views;
pub use views::*;

pub use errors::CollateralError;

use soroban_sdk::contract;

/// Soroban contract root for the collateral domain.

#[contract]
pub struct Collateral;

