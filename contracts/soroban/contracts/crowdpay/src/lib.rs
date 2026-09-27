#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec, token
};

#[cfg(test)]
mod test;

// ---------------------------------------------------------------------------
// Storage TTL constants
// ---------------------------------------------------------------------------

/// Target TTL for instance storage — ~120 days at 5 s/ledger.
///
/// All contract-global state (creator, token, goal, deadline, status, totals,
/// milestones) is kept in instance storage.  Every hot path refreshes the TTL
/// when the remaining lifetime falls below `INSTANCE_TTL_THRESHOLD` so the
/// campaign cannot silently expire mid-flight.
pub const INSTANCE_TTL_EXTEND: u32 = 2_073_600;

/// Refresh threshold for instance storage (half of the extend target).
pub const INSTANCE_TTL_THRESHOLD: u32 = 1_036_800;

/// Target TTL for per-contributor persistent balance entries — same 120-day
/// window so a contributor who deposited near the deadline can still refund
/// after the campaign closes.
pub const PERSISTENT_TTL_EXTEND: u32 = 2_073_600;

/// Refresh threshold for persistent storage (half of the extend target).
pub const PERSISTENT_TTL_THRESHOLD: u32 = 1_036_800;

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Extend instance-storage TTL when it falls below the threshold.
///
/// Called at the start of every mutating entry point so contract-global state
/// remains live throughout any realistically long campaign.
#[inline(always)]
fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND);
}

/// Extend the TTL of a single persistent balance entry when it falls below
/// the threshold.  Called whenever a contributor's balance is read or written
/// on a hot financial path.
#[inline(always)]
fn bump_persistent(env: &Env, key: &DataKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, PERSISTENT_TTL_THRESHOLD, PERSISTENT_TTL_EXTEND);
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Milestone {
    pub target: i128,
    pub released: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    CampaignId,
    Goal,
    Deadline,
    Creator,
    Token,
    TotalRaised,
    Contributions(Address),
    Milestones,
    Status,
}

#[contract]
pub struct CrowdPayContract;

#[contractimpl]
impl CrowdPayContract {
    pub fn initialize(
        env: Env,
        campaign_id: Symbol,
        creator: Address,
        token: Address,
        goal: i128,
        deadline: u64,
        milestones: Vec<Milestone>,
    ) {
        if env.storage().instance().has(&DataKey::CampaignId) {
            panic!("Already initialized");
        }
        env.storage().instance().set(&DataKey::CampaignId, &campaign_id);
        env.storage().instance().set(&DataKey::Creator, &creator);
        env.storage().instance().set(&DataKey::Token, &token);
        env.storage().instance().set(&DataKey::Goal, &goal);
        env.storage().instance().set(&DataKey::Deadline, &deadline);
        env.storage().instance().set(&DataKey::Milestones, &milestones);
        env.storage().instance().set(&DataKey::TotalRaised, &0i128);
        env.storage().instance().set(&DataKey::Status, &symbol_short!("Active"));

        // Stamp the initial TTL so the campaign is live from the first ledger.
        bump_instance(&env);
    }

    pub fn contribute(env: Env, contributor: Address, amount: i128) {
        contributor.require_auth();

        // Refresh instance TTL at the top of every deposit so the campaign's
        // global state (status, deadline, totals) cannot silently expire while
        // contributions are still flowing.
        bump_instance(&env);

        let status: Symbol = env.storage().instance().get(&DataKey::Status).unwrap();
        if status != symbol_short!("Active") && status != symbol_short!("Funded") {
            panic!("Campaign is not accepting contributions");
        }

        let deadline: u64 = env.storage().instance().get(&DataKey::Deadline).unwrap();
        if env.ledger().timestamp() >= deadline {
            panic!("Deadline passed");
        }

        let token_addr: Address = env.storage().instance().get(&DataKey::Token).unwrap();
        let client = token::Client::new(&env, &token_addr);
        client.transfer(&contributor, &env.current_contract_address(), &amount);

        let balance_key = DataKey::Contributions(contributor.clone());
        let current_balance: i128 = env.storage().persistent().get(&balance_key).unwrap_or(0);
        env.storage().persistent().set(&balance_key, &(current_balance + amount));
        // Keep the contributor balance entry alive at least as long as the
        // full refund window (deadline + persistent TTL).
        bump_persistent(&env, &balance_key);

        let total_raised: i128 = env.storage().instance().get(&DataKey::TotalRaised).unwrap_or(0);
        let new_total = total_raised + amount;
        env.storage().instance().set(&DataKey::TotalRaised, &new_total);

        // Check if goal reached
        let goal: i128 = env.storage().instance().get(&DataKey::Goal).unwrap();
        if new_total >= goal && status == symbol_short!("Active") {
            env.storage().instance().set(&DataKey::Status, &symbol_short!("Funded"));
        }
    }

    pub fn release_milestone(env: Env, milestone_index: u32) {
        let creator: Address = env.storage().instance().get(&DataKey::Creator).unwrap();
        creator.require_auth();

        // Refresh instance TTL: milestone release is a high-value financial
        // event and the contract must remain live to record the state change.
        bump_instance(&env);

        let mut milestones: Vec<Milestone> = env.storage().instance().get(&DataKey::Milestones).unwrap();
        let mut milestone = milestones.get(milestone_index).expect("Invalid milestone index");

        if milestone.released {
            panic!("Milestone already released");
        }

        let total_raised: i128 = env.storage().instance().get(&DataKey::TotalRaised).unwrap_or(0);
        if total_raised < milestone.target {
            panic!("Milestone target not met");
        }

        milestone.released = true;
        milestones.set(milestone_index, milestone);
        env.storage().instance().set(&DataKey::Milestones, &milestones);

        let token_addr: Address = env.storage().instance().get(&DataKey::Token).unwrap();
        let client = token::Client::new(&env, &token_addr);
        let balance = client.balance(&env.current_contract_address());

        if balance > 0 {
            client.transfer(&env.current_contract_address(), &creator, &balance);
        }
    }

    pub fn refund(env: Env, contributor: Address) {
        contributor.require_auth();

        // Refresh instance TTL before any state read so campaign config cannot
        // expire between the deadline passing and a contributor claiming their
        // refund.
        bump_instance(&env);

        let status: Symbol = env.storage().instance().get(&DataKey::Status).unwrap();
        if status != symbol_short!("Failed") {
            panic!("Campaign has not failed");
        }

        let balance_key = DataKey::Contributions(contributor.clone());

        // Extend the persistent entry TTL before reading so the lookup cannot
        // fail with an expiry-induced miss even if many ledgers have elapsed
        // since the last deposit.
        bump_persistent(&env, &balance_key);

        let amount: i128 = env.storage().persistent().get(&balance_key).unwrap_or(0);

        if amount <= 0 {
            panic!("No contribution to refund");
        }

        let token_addr: Address = env.storage().instance().get(&DataKey::Token).unwrap();
        let client = token::Client::new(&env, &token_addr);
        client.transfer(&env.current_contract_address(), &contributor, &amount);

        env.storage().persistent().set(&balance_key, &0i128);
        // No need to re-extend after zeroing — the entry will expire naturally.
    }

    pub fn set_failed(env: Env) {
        bump_instance(&env);

        let deadline: u64 = env.storage().instance().get(&DataKey::Deadline).unwrap();
        let total_raised: i128 = env.storage().instance().get(&DataKey::TotalRaised).unwrap_or(0);
        let goal: i128 = env.storage().instance().get(&DataKey::Goal).unwrap();

        if env.ledger().timestamp() >= deadline && total_raised < goal {
            env.storage().instance().set(&DataKey::Status, &symbol_short!("Failed"));
        } else {
            let creator: Address = env.storage().instance().get(&DataKey::Creator).unwrap();
            creator.require_auth();
            env.storage().instance().set(&DataKey::Status, &symbol_short!("Failed"));
        }
    }

    pub fn get_status(env: Env) -> Symbol {
        env.storage().instance().get(&DataKey::Status).unwrap_or(symbol_short!("Active"))
    }

    pub fn get_total_raised(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::TotalRaised).unwrap_or(0)
    }
}
