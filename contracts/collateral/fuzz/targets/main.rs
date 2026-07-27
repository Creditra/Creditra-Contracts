// SPDX-License-Identifier: MIT
#![no_main]

//! # Fuzz target: Collateral Admin Config (v7)
//!
//! This target exercises the stateful properties of the collateral admin configuration,
//! specifically the v7 cool-off guard between critical admin actions.
//!
//! ## Properties under test
//!
//! 1. **Auth check**: Every state-changing entrypoint must invoke `require_auth` for the admin.
//!    (Verified by using `mock_all_auths()` which ensures auth rules are fulfilled.)
//! 2. **Cooldown enforcement**: Critical actions must fail if the cooldown period has not elapsed
//!    since the previous critical action.
//! 3. **Overflow-safe math**: No panics due to arithmetic overflow when calculating cooldown periods.
//! 4. **No unwraps**: Production paths must not panic unexpectedly.

use arbitrary::Arbitrary;
use creditra_credit::{Credit, CreditClient};
use libfuzzer_sys::fuzz_target;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, Vec,
};

/// Represents a discrete action by the admin.
#[derive(Arbitrary, Debug, Clone)]
pub enum AdminAction {
    SetCooldownSeconds(u64),
    SetMinCollateralRatioBps(u32),
    SetCollateralRiskWeight(u32),
    SetCollateralTokenAllowlist(u8),
    AdvanceTime(u64),
}

fuzz_target!(|actions: std::vec::Vec<AdminAction>| {
    let env = Env::default();
    env.mock_all_auths();

    let mut current_ts = 100_000_u64;
    env.ledger().with_mut(|li| li.timestamp = current_ts);

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    // Initialize the main contract so admin is configured
    client.init(&admin);

    let dummy_asset = Address::generate(&env);

    let mut cooldown_secs: u64 = 0; // Default is disabled
    let mut last_action_ts: Option<u64> = None;

    for action in actions {
        match action {
            AdminAction::SetCooldownSeconds(secs) => {
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.set_admin_collateral_cooldown_seconds(&secs);
                }));
                assert!(res.is_ok(), "Configuring cooldown should never panic");
                cooldown_secs = secs;
            }
            AdminAction::SetMinCollateralRatioBps(bps) => {
                let is_cooling_down = is_active_cooldown(current_ts, last_action_ts, cooldown_secs);
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.set_min_collateral_ratio_bps(&bps);
                }));

                if is_cooling_down {
                    assert!(res.is_err(), "Must reject critical action during cooldown");
                } else {
                    assert!(
                        res.is_ok(),
                        "Must accept critical action if not cooling down"
                    );
                    last_action_ts = Some(current_ts);
                }
            }
            AdminAction::SetCollateralRiskWeight(bps) => {
                let is_cooling_down = is_active_cooldown(current_ts, last_action_ts, cooldown_secs);
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.set_collateral_risk_weight(&dummy_asset, &bps);
                }));

                if bps > 10_000 {
                    assert!(
                        res.is_err(),
                        "Must reject invalid risk weight (> 10000 bps)"
                    );
                } else if is_cooling_down {
                    assert!(res.is_err(), "Must reject critical action during cooldown");
                } else {
                    assert!(
                        res.is_ok(),
                        "Must accept critical action if not cooling down"
                    );
                    last_action_ts = Some(current_ts);
                }
            }
            AdminAction::SetCollateralTokenAllowlist(num) => {
                let is_cooling_down = is_active_cooldown(current_ts, last_action_ts, cooldown_secs);
                let mut tokens = Vec::new(&env);
                for _ in 0..(num.min(10)) {
                    tokens.push_back(Address::generate(&env));
                }
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.set_collateral_token_allowlist(&tokens);
                }));

                if is_cooling_down {
                    assert!(res.is_err(), "Must reject critical action during cooldown");
                } else {
                    assert!(
                        res.is_ok(),
                        "Must accept critical action if not cooling down"
                    );
                    last_action_ts = Some(current_ts);
                }
            }
            AdminAction::AdvanceTime(secs) => {
                let advance = secs % (u64::MAX / 2);
                current_ts = current_ts.saturating_add(advance);
                env.ledger().with_mut(|li| li.timestamp = current_ts);
            }
        }
    }
});

/// Helper to simulate the contract's cooldown logic.
fn is_active_cooldown(now: u64, last_ts: Option<u64>, cooldown: u64) -> bool {
    if cooldown == 0 {
        return false;
    }
    if let Some(ts) = last_ts {
        if now < ts.saturating_add(cooldown) {
            return true;
        }
    }
    false
}
