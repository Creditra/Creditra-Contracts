// SPDX-License-Identifier: MIT

//! Authorization utilities for admin-only operations.
//!
//! # Storage
//! - **Admin address**: Instance storage (shared TTL with all instance keys)
//!   - Key: `Symbol("admin")`
//!   - Value: `Address`
//!   - Written once during `init()`, never modified except via admin rotation

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
/// Panics with "admin not set" if the admin key has never been initialized.
pub fn require_admin(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&admin_key(env))
        .expect("admin not set")
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
