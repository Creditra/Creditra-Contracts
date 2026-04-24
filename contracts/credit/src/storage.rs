use crate::types::ContractError;
use soroban_sdk::{contracttype, Address, Env, Symbol};

/// Storage keys used in instance and persistent storage.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    /// Address of the liquidity token (SAC or compatible token contract).
    LiquidityToken,
    /// Address of the liquidity source / reserve that funds draws.
    LiquiditySource,
    /// Optional per-transaction draw cap (admin-configurable).
    MaxDrawAmount,
    /// Persistent flag indicating a borrower is blocked from drawing credit.
    BlockedBorrower(soroban_sdk::Address),
}

pub fn admin_key(env: &Env) -> Symbol {
    Symbol::new(env, "admin")
}

pub fn proposed_admin_key(env: &Env) -> Symbol {
    Symbol::new(env, "proposed_admin")
}

pub fn proposed_at_key(env: &Env) -> Symbol {
    Symbol::new(env, "proposed_at")
}

pub fn reentrancy_key(env: &Env) -> Symbol {
    Symbol::new(env, "reentrancy")
}

pub fn rate_cfg_key(env: &Env) -> Symbol {
    Symbol::new(env, "rate_cfg")
}

/// Instance storage key for the risk-score-based rate formula configuration.
pub fn rate_formula_key(env: &Env) -> Symbol {
    Symbol::new(env, "rate_form")
}

/// Instance storage key for the grace period policy configuration.
pub fn grace_period_key(env: &Env) -> Symbol {
    Symbol::new(env, "grace_cfg")
}

/// Assert reentrancy guard is not set; set it for the duration of the call.
///
/// Panics with [`ContractError::Reentrancy`] if the guard is already active,
/// indicating a reentrant call. Caller **must** call [`clear_reentrancy_guard`]
/// on every success and failure path to release the guard.
pub fn set_reentrancy_guard(env: &Env) {
    let key = reentrancy_key(env);
    let current: bool = env.storage().instance().get(&key).unwrap_or(false);
    if current {
        env.panic_with_error(ContractError::Reentrancy);
    }
    env.storage().instance().set(&key, &true);
}

/// Clear the reentrancy guard set by [`set_reentrancy_guard`].
///
/// Must be called on every exit path (success and failure) of any function
/// that called [`set_reentrancy_guard`].
pub fn clear_reentrancy_guard(env: &Env) {
    env.storage().instance().set(&reentrancy_key(env), &false);
}

/// Check whether a borrower is blocked from drawing credit.
pub fn is_borrower_blocked(env: &Env, borrower: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&DataKey::BlockedBorrower(borrower.clone()))
        .unwrap_or(false)
}

/// Set or clear the blocked status for a borrower.
pub fn set_borrower_blocked(env: &Env, borrower: &Address, blocked: bool) {
    env.storage()
        .persistent()
        .set(&DataKey::BlockedBorrower(borrower.clone()), &blocked);
}
