// SPDX-License-Identifier: MIT

//! Contract initialization and configuration helpers.
//!
//! # What
//!
//! - [`init`] — one-time initialization. Sets the admin address, the
//!   default liquidity source (the contract's own address), the schema
//!   version, the initial global accumulators (`CreditLineCount = 0`,
//!   `TotalUtilized = 0`), and a default minimum collateral ratio of
//!   15 000 bps (150 %) — i.e. the contract ships in a conservative
//!   collateral-required mode and is loosened by admin policy.
//! - [`set_liquidity_token`] — admin sets the SAC / token contract used
//!   for `transfer` and `transfer_from` operations on draw, repay, and
//!   collateral movement.
//! - [`set_liquidity_source`] — admin sets the reserve address that
//!   funds draws. Defaults to the credit contract's own address; a
//!   production deployment typically points this at a separate reserve
//!   pool contract.
//!
//! # How
//!
//! `init`'s re-init guard checks `Symbol("admin")` presence in instance
//! storage. A second `init` call reverts with
//! [`ContractError::AlreadyInitialized`]; the admin address therefore
//! cannot be overwritten by re-initialization, only by the two-step
//! rotation in [`crate::lib::propose_admin`] /
//! [`crate::lib::accept_admin`].
//!
//! # Why (deployment-safe defaults)
//!
//! Shipping `init` with conservative defaults — 150 % collateral floor,
//! contract-as-its-own-reserve-source, schema version 1 — means a
//! freshly deployed contract is immediately safe to attach to a
//! liquidity token without exposure to the protocol's untuned risk
//! parameters. The admin then dials in the rate formula, exposure caps,
//! and so on before opening the first credit line.
//!
//! See [`docs/deploy.md`](../../../docs/deploy.md) for the required
//! deployment sequence and
//! [`docs/EXECUTION_QUALITY.md`](../../../docs/EXECUTION_QUALITY.md) §6
//! for the full testnet / mainnet checklist.

use crate::auth::require_admin_auth;
use crate::storage::{admin_key, set_schema_version, DataKey};
use crate::types::ContractError;
use soroban_sdk::{Address, Env};
use crate::types::ContractError;

/// Initialize the contract exactly once.
pub fn init(env: Env, admin: Address) {
    if env.storage().instance().has(&admin_key(&env)) {
        env.panic_with_error(ContractError::AlreadyInitialized);
    }
    env.storage().instance().set(&admin_key(&env), &admin);
    env.storage()
        .instance()
        .set(&DataKey::LiquiditySource, &env.current_contract_address());

    // Initialize global counters and schema marker.
    env.storage()
        .instance()
        .set(&DataKey::CreditLineCount, &0_u32);
    env.storage()
        .instance()
        .set(&DataKey::TotalUtilized, &0_i128);
    set_schema_version(&env, crate::SCHEMA_VERSION);
    // Set default minimum collateral ratio to 150% (15000 bps)
    crate::storage::set_min_collateral_ratio_bps(&env, 15000);
    // Set default protocol fee bounds: 0 – 1000 bps (10%)
    crate::storage::set_min_protocol_fee_bps(&env, 0);
    crate::storage::set_max_protocol_fee_bps(&env, 1000);
}

/// @notice Sets the token contract used for reserve/liquidity checks and draw transfers.
/// @dev Admin-only.
#[allow(dead_code)]
pub fn set_liquidity_token(env: Env, token_address: Address) {
    require_admin_auth(&env);
    env.storage()
        .instance()
        .set(&DataKey::LiquidityToken, &token_address);
}

/// @notice Sets the address that provides liquidity for draw operations.
/// @dev Admin-only. If unset, init config uses the contract address.
#[allow(dead_code)]
pub fn set_liquidity_source(env: Env, reserve_address: Address) {
    require_admin_auth(&env);
    env.storage()
        .instance()
        .set(&DataKey::LiquiditySource, &reserve_address);
}
