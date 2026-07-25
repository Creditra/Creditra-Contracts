// SPDX-License-Identifier: MIT
#![cfg_attr(not(test), no_std)]
#![allow(clippy::unused_unit)]

// Module declarations
mod accrual;
mod attestation;
mod auth;
pub mod borrow;
mod collateral;
mod config;
pub mod events;
mod fees;
mod freeze;
mod handshake;
pub mod limits;
pub mod math_utils;
mod oracles;
mod penalties;
mod query;
mod risk;
mod scoring;
mod storage;
pub mod types;
mod views;

// Re-exports
pub use crate::risk::compute_rate_from_score;
pub use crate::types::FreezeReason;

use soroban_sdk::{contract, contractimpl, Address, Env, Symbol};
use crate::auth::require_admin_auth;
use crate::events::*;
use crate::storage::*;
use crate::types::*;

#[contract]
pub struct Credit;

#[contractimpl]
impl Credit {
    pub fn get_version() -> (u32, u32, u32) {
        (1, 0, 0)
    }
}