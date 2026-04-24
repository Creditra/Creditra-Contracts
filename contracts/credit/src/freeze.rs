// SPDX-License-Identifier: MIT

//! Global draw-freeze switch.
//!
//! Provides an admin-only emergency control that blocks **all** `draw_credit`
//! calls contract-wide while liquidity reserve operations are underway.
//!
//! # Design
//! - Stored as a single `bool` under [`DataKey::DrawsFrozen`] in instance storage.
//! - Defaults to `false` (draws allowed) when the key is absent.
//! - Distinct from per-line [`CreditStatus::Suspended`]: this flag does not
//!   mutate any borrower's credit line and can be toggled in O(1) regardless
//!   of the number of open lines.
//! - Repayments are **never** blocked by this flag.
//!
//! # Threat model
//! An attacker who gains admin credentials could freeze draws to disrupt
//! borrowers. This is mitigated by the same admin-key security requirements
//! that protect all other admin operations. The flag is intentionally
//! transparent: the current state is readable by anyone via `is_draws_frozen`.

use crate::auth::require_admin_auth;
use crate::events::{publish_draws_frozen_event, DrawsFrozenEvent};
use crate::storage::DataKey;
use soroban_sdk::Env;

/// Freeze all draws globally (admin only).
///
/// Sets [`DataKey::DrawsFrozen`] to `true`. Idempotent: calling when already
/// frozen is a no-op (no event emitted for the redundant call).
///
/// # Events
/// Emits [`DrawsFrozenEvent`] with `frozen = true`.
pub fn freeze_draws(env: Env) {
    let admin = require_admin_auth(&env);
    env.storage().instance().set(&DataKey::DrawsFrozen, &true);
    publish_draws_frozen_event(
        &env,
        DrawsFrozenEvent {
            frozen: true,
            timestamp: env.ledger().timestamp(),
            actor: admin,
        },
    );
}

/// Unfreeze draws globally (admin only).
///
/// Sets [`DataKey::DrawsFrozen`] to `false`. Idempotent: calling when already
/// unfrozen is a no-op (no event emitted for the redundant call).
///
/// # Events
/// Emits [`DrawsFrozenEvent`] with `frozen = false`.
pub fn unfreeze_draws(env: Env) {
    let admin = require_admin_auth(&env);
    env.storage().instance().set(&DataKey::DrawsFrozen, &false);
    publish_draws_frozen_event(
        &env,
        DrawsFrozenEvent {
            frozen: false,
            timestamp: env.ledger().timestamp(),
            actor: admin,
        },
    );
}

/// Returns `true` when draws are globally frozen.
///
/// Defaults to `false` (draws allowed) if the key has never been set.
pub fn is_draws_frozen(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::DrawsFrozen)
        .unwrap_or(false)
}
