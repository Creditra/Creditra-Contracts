// SPDX-License-Identifier: MIT

//! Structured event definitions and publisher functions for the Creditra Collateral contract.
//!
//! # Purpose
//!
//! Exposes a consistent, well-documented event layer that emits structured events
//! whenever collateral state transitions occur. Integrators, indexers, and off-chain
//! monitoring systems can consume these events to track user collateral balances,
//! deposits, withdrawals, releases, liquidations, and transfers.
//!
//! # Topic Conventions
//!
//! Event topics are encoded using `symbol_short!` (≤ 9 characters) for efficient
//! on-chain `SCV_SYMBOL` encoding. All collateral events use `("collat", "<operation>")`
//! as their topic tuple.

use soroban_sdk::{contracttype, symbol_short, Address, Env};

/// Event payload emitted when a user deposits collateral into the contract.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollateralDepositedEvent {
    /// Account address of the user depositing collateral.
    pub user: Address,
    /// Amount of collateral deposited in this transaction.
    pub amount: i128,
    /// Resulting persistent collateral balance for the user after the deposit.
    pub new_balance: i128,
}

/// Event payload emitted when a user withdraws collateral from the contract.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollateralWithdrawnEvent {
    /// Account address of the user withdrawing collateral.
    pub user: Address,
    /// Amount of collateral withdrawn in this transaction.
    pub amount: i128,
    /// Resulting persistent collateral balance for the user after the withdrawal.
    pub new_balance: i128,
}

/// Event payload emitted when a user's collateral balance is updated.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollateralUpdatedEvent {
    /// Account address of the user whose collateral balance was updated.
    pub user: Address,
    /// Previous collateral balance before the update.
    pub old_balance: i128,
    /// New collateral balance after the update.
    pub new_balance: i128,
}

/// Event payload emitted when a portion of collateral is released.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollateralReleasedEvent {
    /// Account address of the user whose collateral was released.
    pub user: Address,
    /// Amount of collateral released.
    pub amount: i128,
    /// Resulting persistent collateral balance for the user after release.
    pub new_balance: i128,
}

/// Event payload emitted when collateral is liquidated.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollateralLiquidatedEvent {
    /// Account address of the user whose collateral was liquidated.
    pub user: Address,
    /// Account address of the liquidator performing the liquidation.
    pub liquidator: Address,
    /// Amount of collateral liquidated.
    pub amount: i128,
    /// Resulting collateral balance for the user after liquidation.
    pub new_balance: i128,
}

/// Event payload emitted when collateral is transferred between accounts.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollateralTransferredEvent {
    /// Account address transferring collateral.
    pub from: Address,
    /// Account address receiving collateral.
    pub to: Address,
    /// Amount of collateral transferred.
    pub amount: i128,
}

/// Event payload emitted when a user's collateral position is closed or removed.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollateralClosedEvent {
    /// Account address whose collateral position was closed.
    pub user: Address,
    /// Final collateral balance at time of closure (typically zero).
    pub final_balance: i128,
}

/// Publish a collateral deposited event.
///
/// # Arguments
/// * `env` - Reference to the Soroban execution environment.
/// * `user` - Account address of the depositor.
/// * `amount` - Amount of collateral deposited.
/// * `new_balance` - Resulting persistent collateral balance.
pub fn publish_collateral_deposited(env: &Env, user: &Address, amount: i128, new_balance: i128) {
    env.events().publish(
        (symbol_short!("collat"), symbol_short!("deposit")),
        CollateralDepositedEvent {
            user: user.clone(),
            amount,
            new_balance,
        },
    );
}

/// Publish a collateral withdrawn event.
///
/// # Arguments
/// * `env` - Reference to the Soroban execution environment.
/// * `user` - Account address of the withdrawer.
/// * `amount` - Amount of collateral withdrawn.
/// * `new_balance` - Resulting persistent collateral balance.
pub fn publish_collateral_withdrawn(env: &Env, user: &Address, amount: i128, new_balance: i128) {
    env.events().publish(
        (symbol_short!("collat"), symbol_short!("withdraw")),
        CollateralWithdrawnEvent {
            user: user.clone(),
            amount,
            new_balance,
        },
    );
}

/// Publish a collateral updated event.
///
/// # Arguments
/// * `env` - Reference to the Soroban execution environment.
/// * `user` - Account address of the user.
/// * `old_balance` - Balance prior to the update.
/// * `new_balance` - Balance following the update.
pub fn publish_collateral_updated(
    env: &Env,
    user: &Address,
    old_balance: i128,
    new_balance: i128,
) {
    env.events().publish(
        (symbol_short!("collat"), symbol_short!("updated")),
        CollateralUpdatedEvent {
            user: user.clone(),
            old_balance,
            new_balance,
        },
    );
}

/// Publish a collateral released event.
///
/// # Arguments
/// * `env` - Reference to the Soroban execution environment.
/// * `user` - Account address of the user.
/// * `amount` - Amount of collateral released.
/// * `new_balance` - Resulting collateral balance.
pub fn publish_collateral_released(env: &Env, user: &Address, amount: i128, new_balance: i128) {
    env.events().publish(
        (symbol_short!("collat"), symbol_short!("release")),
        CollateralReleasedEvent {
            user: user.clone(),
            amount,
            new_balance,
        },
    );
}

/// Publish a collateral liquidated event.
///
/// # Arguments
/// * `env` - Reference to the Soroban execution environment.
/// * `user` - Account address of the liquidated position owner.
/// * `liquidator` - Account address of the liquidator.
/// * `amount` - Amount of collateral liquidated.
/// * `new_balance` - Resulting collateral balance.
pub fn publish_collateral_liquidated(
    env: &Env,
    user: &Address,
    liquidator: &Address,
    amount: i128,
    new_balance: i128,
) {
    env.events().publish(
        (symbol_short!("collat"), symbol_short!("liquidate")),
        CollateralLiquidatedEvent {
            user: user.clone(),
            liquidator: liquidator.clone(),
            amount,
            new_balance,
        },
    );
}

/// Publish a collateral transferred event.
///
/// # Arguments
/// * `env` - Reference to the Soroban execution environment.
/// * `from` - Account address transferring collateral.
/// * `to` - Account address receiving collateral.
/// * `amount` - Amount transferred.
pub fn publish_collateral_transferred(env: &Env, from: &Address, to: &Address, amount: i128) {
    env.events().publish(
        (symbol_short!("collat"), symbol_short!("transfer")),
        CollateralTransferredEvent {
            from: from.clone(),
            to: to.clone(),
            amount,
        },
    );
}

/// Publish a collateral closed event.
///
/// # Arguments
/// * `env` - Reference to the Soroban execution environment.
/// * `user` - Account address of the position owner.
/// * `final_balance` - Final collateral balance.
pub fn publish_collateral_closed(env: &Env, user: &Address, final_balance: i128) {
    env.events().publish(
        (symbol_short!("collat"), symbol_short!("closed")),
        CollateralClosedEvent {
            user: user.clone(),
            final_balance,
        },
    );
}
