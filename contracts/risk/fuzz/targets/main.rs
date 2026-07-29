// SPDX-License-Identifier: MIT
#![no_main]

//! # Fuzz Target: Risk Admin Cooldown (v7)
//!
//! This target exercises the stateful properties and security invariants of the [`creditra_risk`]
//! contract, specifically focusing on the v7 risk admin cool-off guard between critical admin
//! actions, pause/unpause circuit breaker states, authorization boundaries, and arithmetic overflow safety.
//!
//! ## Invariants Under Test
//!
//! 1. **Auth Enforcement**: Every state-changing entrypoint (`init`, `set_risk_admin_cooldown`,
//!    `set_paused`, `record_risk_admin_action`) requires valid admin authorization.
//! 2. **Cooldown Enforcement**: When a non-zero cooldown is configured (`seconds > 0`) and a prior
//!    action has been recorded (`last_action_ts > 0`), any subsequent call to `record_risk_admin_action`
//!    before `last_action_ts + cooldown_seconds` MUST be rejected with `RiskAdminCooldownActive`.
//! 3. **Pause Circuit Breaker**: When the contract is paused, state-changing risk mutations
//!    (`set_risk_admin_cooldown`, `record_risk_admin_action`) MUST panic with `Paused`,
//!    whereas read-only queries (`get_risk_admin_cooldown`, `get_admin`) and unpausing succeed.
//! 4. **First Action Invariant**: The initial call to `record_risk_admin_action` always succeeds
//!    (provided unpaused and authorized), regardless of the configured cooldown duration.
//! 5. **Overflow Safety**: Timestamp operations and arithmetic comparisons use saturating math
//!    and do not overflow under arbitrary `u64` values or timestamp jumps.
//! 6. **TTL Hygiene**: Contract calls maintain valid instance storage TTL without panicking.

use arbitrary::Arbitrary;
use creditra_risk::{RiskContract, RiskContractClient};
use libfuzzer_sys::fuzz_target;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env,
};

/// Represents a discrete action by an admin or unauthorized actor on the Risk contract.
#[derive(Arbitrary, Debug, Clone)]
pub enum RiskAction {
    /// Configure the risk admin cooldown duration in seconds.
    SetCooldownSeconds(u64),
    /// Set the paused state of the contract.
    SetPaused(bool),
    /// Record a risk admin action timestamp.
    RecordAction,
    /// Query the configured cooldown duration.
    GetCooldown,
    /// Query the configured admin address.
    GetAdmin,
    /// Advance the ledger timestamp by a given delta.
    AdvanceTime(u64),
    /// Attempt an action with an unauthorized (non-admin) caller.
    UnauthorizedAction(u8),
}

fuzz_target!(|actions: std::vec::Vec<RiskAction>| {
    let env = Env::default();
    env.mock_all_auths();

    let mut current_ts: u64 = 100_000;
    env.ledger().with_mut(|li| li.timestamp = current_ts);

    let admin = Address::generate(&env);
    let contract_id = env.register(RiskContract, ());
    let client = RiskContractClient::new(&env, &contract_id);

    // Initialize the Risk contract with admin address
    client.init(&admin);

    let mut cooldown_secs: u64 = 0;
    let mut last_action_ts: u64 = 0;
    let mut is_paused: bool = false;

    for action in actions {
        match action {
            RiskAction::SetCooldownSeconds(secs) => {
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.set_risk_admin_cooldown(&secs);
                }));

                if is_paused {
                    assert!(res.is_err(), "set_risk_admin_cooldown must fail when paused");
                } else {
                    assert!(res.is_ok(), "set_risk_admin_cooldown must succeed when authorized and not paused");
                    cooldown_secs = secs;
                    assert_eq!(client.get_risk_admin_cooldown(), secs, "get_risk_admin_cooldown must match set value");
                }
            }
            RiskAction::SetPaused(paused) => {
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.set_paused(&paused);
                }));
                assert!(res.is_ok(), "set_paused must succeed for admin");
                is_paused = paused;
            }
            RiskAction::RecordAction => {
                let is_cooling_down = is_active_cooldown(current_ts, last_action_ts, cooldown_secs);
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.record_risk_admin_action();
                }));

                if is_paused {
                    assert!(res.is_err(), "record_risk_admin_action must fail when paused");
                } else if is_cooling_down {
                    assert!(res.is_err(), "record_risk_admin_action must fail during active cooldown");
                } else {
                    assert!(res.is_ok(), "record_risk_admin_action must succeed when not paused and cooldown elapsed");
                    last_action_ts = current_ts;
                }
            }
            RiskAction::GetCooldown => {
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.get_risk_admin_cooldown()
                }));
                assert!(res.is_ok(), "get_risk_admin_cooldown must never panic");
                if !is_paused {
                    assert_eq!(res.unwrap(), cooldown_secs);
                }
            }
            RiskAction::GetAdmin => {
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.get_admin()
                }));
                assert!(res.is_ok(), "get_admin must never panic when initialized");
                assert_eq!(res.unwrap(), admin);
            }
            RiskAction::AdvanceTime(delta) => {
                let advance = delta % (u64::MAX / 4);
                current_ts = current_ts.saturating_add(advance);
                env.ledger().with_mut(|li| li.timestamp = current_ts);
            }
            RiskAction::UnauthorizedAction(variant) => {
                env.mock_all_auths_allowing_non_root_auth();
                let non_admin = Address::generate(&env);
                non_admin.require_auth();

                match variant % 3 {
                    0 => {
                        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            client.set_risk_admin_cooldown(&1000);
                        }));
                        assert!(res.is_err(), "set_risk_admin_cooldown must reject non-admin caller");
                    }
                    1 => {
                        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            client.set_paused(&true);
                        }));
                        assert!(res.is_err(), "set_paused must reject non-admin caller");
                    }
                    _ => {
                        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            client.record_risk_admin_action();
                        }));
                        assert!(res.is_err(), "record_risk_admin_action must reject non-admin caller");
                    }
                }

                env.mock_all_auths();
            }
        }
    }
});

/// Helper to evaluate if the risk admin cooldown is currently active based on timestamp.
fn is_active_cooldown(now: u64, last_ts: u64, cooldown: u64) -> bool {
    if cooldown == 0 || last_ts == 0 {
        return false;
    }
    now < last_ts.saturating_add(cooldown)
}
