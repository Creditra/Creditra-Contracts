// SPDX-License-Identifier: MIT

//! Read-only risk capabilities views for the Creditra credit contract.

use crate::risk::get_rate_change_limits;
use crate::scoring::get_vrf_commitment;
use crate::storage::{
    get_borrow_admin_cooldown, get_credit_line, get_last_borrow_admin_action_ts, is_paused,
};
use crate::types::RiskCapabilities;
use soroban_sdk::{Address, Env};

// ── Risk capabilities view ───────────────────────────────────────────────────

/// Return a borrower's current risk capabilities bitmap.
///
/// Read-only view for off-chain risk engines and admin tooling. Evaluates the
/// same state-dependent pre-flight checks used by `update_risk_parameters`,
/// `commit_vrf_output`, and rate cadence guards, except for value-dependent
/// validation (proposed limit, rate delta, or score vs VRF commitment).
///
/// # Parameters
/// - `borrower`: The borrower address to query.
///
/// # Returns
/// A [`RiskCapabilities`] struct describing which risk mutations should
/// succeed assuming valid admin authorization and parameters.
pub fn capabilities(env: Env, borrower: Address) -> RiskCapabilities {
    let paused = is_paused(&env);
    let credit_line = get_credit_line(&env, &borrower);

    let cooldown_blocks = borrow_admin_cooldown_blocks(&env, &borrower);

    let can_update_risk_parameters = credit_line
        .as_ref()
        .is_some_and(|_| !paused && !cooldown_blocks);

    let can_change_rate = credit_line
        .as_ref()
        .is_some_and(|line| {
            if paused || cooldown_blocks {
                return false;
            }
            match get_rate_change_limits(env.clone()) {
                None => true,
                Some(cfg) => {
                    if cfg.rate_change_min_interval == 0 || line.last_rate_update_ts == 0 {
                        true
                    } else {
                        let elapsed = env
                            .ledger()
                            .timestamp()
                            .saturating_sub(line.last_rate_update_ts);
                        elapsed >= cfg.rate_change_min_interval
                    }
                }
            }
        });

    let can_commit_vrf = !paused && get_vrf_commitment(&env, &borrower).is_none();

    RiskCapabilities {
        can_update_risk_parameters,
        can_change_rate,
        can_commit_vrf,
    }
}

/// Returns `true` when the configured borrow admin cooldown would reject an update.
fn borrow_admin_cooldown_blocks(env: &Env, borrower: &Address) -> bool {
    let Some(cooldown_seconds) = get_borrow_admin_cooldown(env) else {
        return false;
    };
    if cooldown_seconds == 0 {
        return false;
    }

    let now = env.ledger().timestamp();
    if let Some(last_ts) = get_last_borrow_admin_action_ts(env, borrower) {
        now < last_ts.saturating_add(cooldown_seconds)
    } else {
        false
    }
}
