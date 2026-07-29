// SPDX-License-Identifier: MIT

//! Administrative controls for the borrow contract.
//!
//! This module provides the shared cool-off window used by critical borrow
//! administration actions. The cooldown is global to the contract's admin
//! action stream: a successful critical action delays every other critical
//! admin action until the configured interval has elapsed.

use soroban_sdk::{contracttype, Address, Env};

/// Persistent keys owned by the borrow-admin controls.
///
/// These variants are appended rather than reordered so that their Soroban
/// encodings remain stable across contract upgrades.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum BorrowAdminDataKey {
    /// Configured cool-off duration in seconds.
    CooldownSeconds,
    /// Ledger timestamp of the most recent successful critical action.
    LastActionTimestamp,
}

/// Error returned when a critical admin action is attempted too soon.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BorrowAdminCooldownError {
    /// The configured cool-off interval has not elapsed.
    Active,
}

/// Returns whether a critical action is allowed at `now`.
///
/// A zero cooldown disables the guard. If the ledger timestamp moves
/// backwards, the elapsed duration is treated as zero rather than wrapping.
/// This deliberately avoids `last + cooldown`, whose addition could overflow
/// for adversarially large values.
#[inline]
pub fn cooldown_allows(last_action: Option<u64>, now: u64, cooldown: u64) -> bool {
    if cooldown == 0 {
        return true;
    }

    match last_action {
        None => true,
        Some(last) => now.saturating_sub(last) >= cooldown,
    }
}

/// Sets the borrow-admin cooldown duration.
///
/// The caller must pass the contract's already-authorized administrator. The
/// explicit `require_auth` protects this state-changing operation when it is
/// exposed directly as an entrypoint. Setting `seconds` to zero removes the
/// configured cooldown and preserves backward-compatible behavior.
///
/// This function does not consume a cooldown window; changing configuration is
/// not itself a critical borrow action.
pub fn set_borrow_admin_cooldown(env: &Env, admin: &Address, seconds: u64) {
    admin.require_auth();

    let storage = env.storage().instance();
    if seconds == 0 {
        storage.remove(&BorrowAdminDataKey::CooldownSeconds);
    } else {
        storage.set(&BorrowAdminDataKey::CooldownSeconds, &seconds);
    }
}

/// Returns the configured borrow-admin cooldown, or `None` if disabled.
pub fn get_borrow_admin_cooldown(env: &Env) -> Option<u64> {
    env.storage()
        .instance()
        .get(&BorrowAdminDataKey::CooldownSeconds)
}

/// Returns the timestamp of the most recent successful critical action.
pub fn get_last_borrow_admin_action_timestamp(env: &Env) -> Option<u64> {
    env.storage()
        .instance()
        .get(&BorrowAdminDataKey::LastActionTimestamp)
}

/// Checks whether a critical admin action may proceed.
///
/// Call this before mutating the action's business state. Call
/// [`record_borrow_admin_action`] only after that mutation has completed
/// successfully, so failed actions cannot consume the cooldown window.
pub fn check_borrow_admin_cooldown(
    env: &Env,
) -> Result<(), BorrowAdminCooldownError> {
    let cooldown = get_borrow_admin_cooldown(env).unwrap_or(0);
    let last_action = get_last_borrow_admin_action_timestamp(env);

    if cooldown_allows(last_action, env.ledger().timestamp(), cooldown) {
        Ok(())
    } else {
        Err(BorrowAdminCooldownError::Active)
    }
}

/// Records a successfully completed critical borrow-admin action.
///
/// This function must be called after the associated state mutation has
/// succeeded. It intentionally performs no authorization check because the
/// enclosing administrative entrypoint is responsible for authenticating and
/// authorizing its administrator before invoking the action.
pub fn record_borrow_admin_action(env: &Env) {
    let timestamp = env.ledger().timestamp();
    env.storage()
        .instance()
        .set(&BorrowAdminDataKey::LastActionTimestamp, &timestamp);
}

#[cfg(test)]
mod tests {
    use super::cooldown_allows;

    #[test]
    fn zero_cooldown_allows_successive_actions_at_same_timestamp() {
        assert!(cooldown_allows(Some(100), 100, 0));
    }

    #[test]
    fn first_action_is_allowed_and_boundary_is_inclusive() {
        assert!(cooldown_allows(None, 100, 300));
        assert!(!cooldown_allows(Some(100), 399, 300));
        assert!(cooldown_allows(Some(100), 400, 300));
    }

    #[test]
    fn failed_timestamp_regression_does_not_bypass_cooldown() {
        assert!(!cooldown_allows(Some(500), 100, 1));
    }

    #[test]
    fn large_timestamps_are_checked_without_addition_overflow() {
        assert!(!cooldown_allows(Some(u64::MAX - 10), u64::MAX, 11));
        assert!(cooldown_allows(Some(u64::MAX - 10), u64::MAX, 10));
    }
}
