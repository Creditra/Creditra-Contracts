// SPDX-License-Identifier: MIT

//! Read-only capability view for collateral operations.
//!
//! Exposes a `u64` bitmask of supported collateral features so clients can
//! detect capability deltas across contract versions.

use soroban_sdk::Env;

/// Bit 0 (0x01): Single-asset deposit (`deposit_collateral`).
pub const CAPABILITY_DEPOSIT: u64 = 1 << 0;

/// Bit 1 (0x02): Single-asset withdrawal with ratio guard (`withdraw_collateral`).
pub const CAPABILITY_WITHDRAW: u64 = 1 << 1;

/// Bit 2 (0x04): Partial collateral release with health-factor monitoring (`partial_release_collateral`).
pub const CAPABILITY_PARTIAL_RELEASE: u64 = 1 << 2;

/// Bit 3 (0x08): Multi-token allowlisted collateral operations (`deposit_collateral_token`, `withdraw_collateral_token`).
pub const CAPABILITY_MULTI_TOKEN: u64 = 1 << 3;

/// Bit 4 (0x10): Per-asset risk weighting in basis points (`set_collateral_risk_weight`).
pub const CAPABILITY_RISK_WEIGHTING: u64 = 1 << 4;

/// Bit 5 (0x20): Admin cool-off guard for collateral parameter updates (`set_admin_collateral_cooldown_seconds`).
pub const CAPABILITY_ADMIN_COOLDOWN: u64 = 1 << 5;

/// Bit 6 (0x40): Minimum collateral ratio floor enforcement (`set_min_collateral_ratio_bps`).
pub const CAPABILITY_RATIO_FLOOR: u64 = 1 << 6;

/// Aggregate bitmask of all currently supported collateral capabilities.
pub const ALL_COLLATERAL_CAPABILITIES: u64 = CAPABILITY_DEPOSIT
    | CAPABILITY_WITHDRAW
    | CAPABILITY_PARTIAL_RELEASE
    | CAPABILITY_MULTI_TOKEN
    | CAPABILITY_RISK_WEIGHTING
    | CAPABILITY_ADMIN_COOLDOWN
    | CAPABILITY_RATIO_FLOOR;

/// Return a `u64` bitmap of supported collateral features.
///
/// This is a read-only, non-mutating view function requiring no authorization.
/// Clients can inspect the returned bitmap to verify feature support before
/// invoking specific collateral entrypoints.
///
/// # Parameters
/// - `_env`: Reference to the Soroban environment.
///
/// # Returns
/// A `u64` bitmask containing all active collateral capability flags.
pub fn capabilities(_env: &Env) -> u64 {
    ALL_COLLATERAL_CAPABILITIES
}
