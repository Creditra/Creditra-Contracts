// SPDX-License-Identifier: MIT

//! Authorization utilities for admin-only operations.
//!
//! # What
//!
//! Two tiny helpers — [`require_admin`] (read-only lookup) and
//! [`require_admin_auth`] (lookup + `require_auth()`) — that gate every
//! admin entrypoint in the contract.
//!
//! # How
//!
//! `require_admin` reads `Symbol("admin")` from instance storage and
//! panics with [`crate::types::ContractError::AdminNotInitialized`] if the
//! slot is empty. `require_admin_auth` additionally invokes
//! `admin.require_auth()`, which delegates to the Soroban host's
//! authorization framework — the host verifies that the transaction is
//! signed (or auth-entry attested) by the admin address before the
//! function returns.
//!
//! # Why
//!
//! Concentrating auth here means every admin-gated entrypoint in
//! [`crate::lib`] reads exactly one line — `require_admin_auth(&env)` —
//! to enforce the auth policy. Adding a new admin-gated entrypoint is
//! mechanical and cannot accidentally skip the check.
//!
//! Admin rotation is two-step (`propose_admin` → `accept_admin` with a
//! configurable delay) and is implemented in [`crate::lib`] rather than
//! here; this module only reads the current admin slot.
//!
//! # Storage
//!
//! - **Admin address**: Instance storage (shared TTL with all instance keys).
//!   - Key: `Symbol("admin")`
//!   - Value: `Address`
//!   - Written once during `init()`, never modified except via the
//!     two-step admin rotation in [`crate::lib::propose_admin`] /
//!     [`crate::lib::accept_admin`].
//!
//! See [`docs/threat-model.md`](../../../docs/threat-model.md) for the
//! authorization matrix mapping every entrypoint to its auth requirement.

use crate::storage::admin_key;
use soroban_sdk::{Address, Env};

/// Retrieve the current admin address from instance storage.
///
/// # Storage
/// - **Type**: Instance storage (shared TTL with all instance keys)
/// - **Key**: `Symbol("admin")`
/// - **TTL Note**: Critical for access control — if instance is archived,
///   admin cannot be retrieved and all admin operations will fail.
///   Production deployments must extend instance TTL regularly.
///
/// # Panics
/// Panics with `ContractError::AdminNotInitialized` if the admin key has never been initialized.
pub fn require_admin(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&admin_key(env))
        .unwrap_or_else(|| env.panic_with_error(crate::types::ContractError::AdminNotInitialized))
}

/// Require admin authorization for the current operation.
///
/// Retrieves the admin address and requires their authorization via `require_auth()`.
/// Returns the admin address for use in event emissions or further checks.
///
/// # Storage
/// - **Type**: Instance storage (shared TTL with all instance keys)
/// - **Key**: `Symbol("admin")`
pub fn require_admin_auth(env: &Env) -> Address {
    let admin = require_admin(env);
    admin.require_auth();
    admin
}
