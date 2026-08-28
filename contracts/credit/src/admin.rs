// SPDX-License-Identifier: MIT

//! Admin-only state version management for the credit contract.

use crate::auth::require_admin_auth;
use crate::storage::set_schema_version;
use soroban_sdk::Env;

/// Admin-only entrypoint to stamp the persisted state version.
///
/// This is an explicit version marker for the contract's persisted state. It
/// can be called by the admin after a schema migration to backfill the marker
/// on contracts that were deployed before this feature existed. The function
/// is idempotent; calling it with the current version is a no-op.
pub fn set_persisted_state_version(env: Env, version: u32) {
    require_admin_auth(&env);
    set_schema_version(&env, version);
}
