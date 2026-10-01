// SPDX-License-Identifier: MIT

//! Authorization utilities for admin-only operations.
//!
//! # What
//!
//! Three small helpers — [`require_admin`] (read-only lookup),
//! [`require_admin_auth`] (lookup + `require_auth()`), and
//! [`require_admin_auth_with_argument`] (argument equality check + one auth) —
//! that gate admin entrypoints in the contract.
//!
//! # How
//!
//! `require_admin` reads `Symbol("admin")` from instance storage and
//! panics with [`crate::types::ContractError::AdminNotInitialized`] if the
//! slot is empty. `require_admin_auth` additionally invokes
//! `admin.require_auth()`, which delegates to the Soroban host's
//! authorization framework. Entry points retaining a caller-supplied `admin`
//! argument use `require_admin_auth_with_argument`: it checks that the
//! argument equals the stored admin, returns `Unauthorized` on mismatch, and
//! authorizes the stored admin exactly once.
//!
//! # Why
//!
//! Concentrating auth here means admin-gated entrypoints in [`crate::lib`]
//! use one of these helpers rather than reimplementing authorization checks.
//! Adding a new admin-gated entrypoint is mechanical and cannot accidentally
//! skip the check or authorize an unvalidated admin argument.
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

/// Require that the supplied admin argument is the configured admin and
/// authorize that address exactly once.
///
/// Entrypoints that retain an `admin: Address` argument for API compatibility
/// must not authorize the argument and then independently authorize the
/// configured admin. Besides creating a duplicate auth requirement, that
/// pattern can allow a mismatched argument to request authorization for an
/// unrelated address. Compare first, return `Unauthorized` on mismatch, then
/// authorize only the configured admin.
pub fn require_admin_auth_with_argument(env: &Env, provided_admin: &Address) -> Address {
    let admin = require_admin(env);
    if provided_admin != &admin {
        env.panic_with_error(crate::types::ContractError::Unauthorized);
    }
    admin.require_auth();
    admin
}
