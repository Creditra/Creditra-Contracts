// SPDX-License-Identifier: MIT

//! Read-only capabilities bitmap for the freeze subsystem (v7).
//!
//! # What
//!
//! Exposes [`freeze_capabilities`], a pure read-only view that returns a `u64`
//! bitmask of every freeze feature supported by this contract version, and
//! [`get_state`], a read-only view returning a full [`FreezeState`] snapshot.
//! Clients and off-chain tooling can call these views to detect capability
//! deltas across contract upgrades without simulating any state-changing
//! transaction.
//!
//! # Design
//!
//! The bitmap follows the same `u64`-per-feature-flag pattern established by
//! `contracts/collateral/src/views.rs`. Each capability constant occupies a
//! single bit. The aggregate [`ALL_FREEZE_CAPABILITIES`] constant is the OR
//! of every active bit and is what [`freeze_capabilities`] returns.
//!
//! The function is intentionally stateless: it takes no storage reads and
//! requires no authorization.
//!
//! # Capability table
//!
//! | Constant                           | Bit | Hex    | Entrypoints covered                                  |
//! |------------------------------------|-----|--------|------------------------------------------------------|
//! | [`CAPABILITY_FREEZE_DRAWS`]        | 0   | `0x01` | `freeze_draws`, `unfreeze_draws`                     |
//! | [`CAPABILITY_FREEZE_CREDIT_LINE`]  | 1   | `0x02` | `freeze_credit_line`, `unfreeze_credit_line`         |
//! | [`CAPABILITY_FREEZE_BORROWER`]     | 2   | `0x04` | `freeze_borrower_until`, `unfreeze_borrower`         |
//! | [`CAPABILITY_FREEZE_REASON`]       | 3   | `0x08` | `get_draws_freeze_reason`, `get_credit_line_freeze_reason` |
//! | [`CAPABILITY_BORROWER_EXPIRY`]     | 4   | `0x10` | `get_borrower_frozen_until`, time-bounded freeze     |
//! | [`CAPABILITY_FREEZE_COOLDOWN`]     | 5   | `0x20` | admin cool-off guard on all state-changing freeze ops |
//! | [`CAPABILITY_GET_STATE`]           | 6   | `0x40` | `get_state` full state snapshot view                  |
//!
//! # See also
//! - [`contracts/collateral/src/views.rs`] — canonical bitmap pattern.
//! - [`contracts/freeze/tests/views_capabilities.rs`] — focused unit tests.

use soroban_sdk::{contracttype, Address, Env};

// ── Per-feature capability constants ──────────────────────────────────────

/// Bit 0 (`0x01`): Global draws freeze / unfreeze.
///
/// Set when the contract supports [`freeze_draws`] and [`unfreeze_draws`].
/// Admin authorization is required for both state-changing entrypoints.
pub const CAPABILITY_FREEZE_DRAWS: u64 = 1 << 0;

/// Bit 1 (`0x02`): Per-borrower credit-line freeze / unfreeze.
///
/// Set when the contract supports [`freeze_credit_line`] and
/// [`unfreeze_credit_line`]. Admin authorization is required.
pub const CAPABILITY_FREEZE_CREDIT_LINE: u64 = 1 << 1;

/// Bit 2 (`0x04`): Time-bounded per-borrower freeze.
///
/// Set when the contract supports [`freeze_borrower_until`] and
/// [`unfreeze_borrower`]. The freeze expires automatically when the ledger
/// timestamp advances past the recorded `frozen_until` value.
pub const CAPABILITY_FREEZE_BORROWER: u64 = 1 << 2;

/// Bit 3 (`0x08`): Structured [`FreezeReason`] classification.
///
/// Set when freeze actions record a typed reason and the contract exposes
/// [`get_draws_freeze_reason`] and [`get_credit_line_freeze_reason`]
/// read-only queries. Enables off-chain tooling to surface compliance
/// context without replaying events.
pub const CAPABILITY_FREEZE_REASON: u64 = 1 << 3;

/// Bit 4 (`0x10`): Borrower freeze-until expiry query.
///
/// Set when the contract exposes [`get_borrower_frozen_until`], allowing
/// callers to read the expiry timestamp of a time-bounded freeze without
/// performing a full is-frozen check.
pub const CAPABILITY_BORROWER_EXPIRY: u64 = 1 << 4;

/// Bit 5 (`0x20`): Admin cool-off guard on freeze operations.
///
/// Set when the contract enforces a configurable cooldown between successive
/// state-changing freeze invocations (`freeze_draws`, `freeze_credit_line`,
/// `freeze_borrower_until`). Prevents rapid-fire admin automation from
/// overwhelming on-chain indexers or bypassing rate-limit policies.
pub const CAPABILITY_FREEZE_COOLDOWN: u64 = 1 << 5;

/// Bit 6 (`0x40`): Read-only `get_state` full state snapshot view.
///
/// Set when the contract exposes [`get_state`], a convenience view returning
/// a [`FreezeState`] struct with the contract admin and global freeze flag.
/// Enables off-chain dashboards and indexers to obtain a complete contract
/// state snapshot in a single read without calling `get_admin` +
/// `is_globally_frozen` separately.
pub const CAPABILITY_GET_STATE: u64 = 1 << 6;

// ── Aggregate ─────────────────────────────────────────────────────────────

/// Aggregate bitmask of all currently supported freeze capabilities.
///
/// This is the value returned by [`freeze_capabilities`]. When adding a new
/// capability constant, include it here and update the capability table in
/// this module's top-level rustdoc.
pub const ALL_FREEZE_CAPABILITIES: u64 = CAPABILITY_FREEZE_DRAWS
    | CAPABILITY_FREEZE_CREDIT_LINE
    | CAPABILITY_FREEZE_BORROWER
    | CAPABILITY_FREEZE_REASON
    | CAPABILITY_BORROWER_EXPIRY
    | CAPABILITY_FREEZE_COOLDOWN
    | CAPABILITY_GET_STATE;

// ── View function ──────────────────────────────────────────────────────────

/// Return a `u64` bitmask of all freeze features supported by this contract.
///
/// # What
///
/// Each bit in the returned value corresponds to a named capability constant
/// defined in this module. Clients can test individual bits with the exported
/// `CAPABILITY_*` constants to determine which freeze operations are
/// available before constructing a transaction.
///
/// # Parameters
/// - `_env`: Reference to the Soroban execution environment (unused; present
///   for API symmetry with other capability views).
///
/// # Returns
///
/// A `u64` bitmask equal to [`ALL_FREEZE_CAPABILITIES`].
///
/// # Security
///
/// - **No authentication required** — this is a pure read-only view.
/// - **No state mutations** — no ledger storage is read or written.
/// - **No cross-contract calls** — the value is a compile-time constant.
///
/// # Example
///
/// ```ignore
/// let caps = freeze_capabilities(&env);
/// assert!(caps & CAPABILITY_FREEZE_DRAWS != 0);   // global draws freeze supported
/// assert!(caps & CAPABILITY_FREEZE_BORROWER != 0); // time-bounded freeze supported
/// ```
pub fn freeze_capabilities(_env: &Env) -> u64 {
    ALL_FREEZE_CAPABILITIES
}

// ── FreezeState ───────────────────────────────────────────────────────────

/// Full state snapshot for the freeze contract.
///
/// Returned by [`get_state`] to provide a comprehensive read-only view of the
/// contract's current state: admin address and global freeze status.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FreezeState {
    /// Contract admin address, `None` if the contract has not been initialized.
    pub admin: Option<Address>,
    /// Whether global protocol emergency freeze is active.
    pub global_freeze_active: bool,
}

// ── get_state view ────────────────────────────────────────────────────────

/// Return a full read-only state snapshot for the freeze contract.
///
/// # What
///
/// Assembles the contract admin address and global freeze flag into a single
/// [`FreezeState`] struct, avoiding the need for callers to issue separate
/// `get_admin` and `is_globally_frozen` queries.
///
/// # Parameters
/// - `env`: Soroban execution environment.
///
/// # Returns
///
/// A [`FreezeState`] struct containing:
/// - `admin`: The contract admin address, or `None` if uninitialized.
/// - `global_freeze_active`: `true` when the global emergency freeze is active.
///
/// # Security
///
/// - **No authentication required** — this is a pure read-only view.
/// - **No state mutations** — only instance-storage reads are performed.
/// - **No cross-contract calls** — the value is derived from local storage.
///
/// # Example
///
/// ```ignore
/// let state = get_state(&env);
/// assert_eq!(state.admin, Some(expected_admin));
/// assert_eq!(state.global_freeze_active, false);
/// ```
pub fn get_state(env: &Env) -> FreezeState {
    let admin: Option<Address> = env.storage().instance().get(&crate::DataKey::Admin);
    let global_freeze_active = env
        .storage()
        .instance()
        .get::<_, bool>(&crate::DataKey::GlobalFreeze)
        .unwrap_or(false);

    FreezeState {
        admin,
        global_freeze_active,
    }
}
