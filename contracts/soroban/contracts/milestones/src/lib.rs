#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, Address, BytesN, Env, Symbol, Vec, IntoVal
};

// ---------------------------------------------------------------------------
// Storage TTL constants
// ---------------------------------------------------------------------------

/// Target TTL for instance storage — ~120 days at 5 s/ledger.
///
/// All milestone state (creator, platform, escrow address, milestone list) is
/// kept in instance storage.  Every hot path refreshes the TTL when the
/// remaining lifetime falls below `INSTANCE_TTL_THRESHOLD` so milestone state
/// cannot silently expire while a campaign is progressing through its payout
/// schedule.
pub const INSTANCE_TTL_EXTEND: u32 = 2_073_600;

/// Refresh threshold for instance storage (half of the extend target).
/// Keeping the threshold at 50 % prevents unnecessary extend calls on every
/// single invocation while still leaving a generous safety margin.
pub const INSTANCE_TTL_THRESHOLD: u32 = 1_036_800;

// ---------------------------------------------------------------------------
// Internal helper
// ---------------------------------------------------------------------------

/// Extend instance-storage TTL when it falls below the threshold.
///
/// The milestones contract stores everything in instance storage (there are no
/// per-contributor persistent entries), so a single helper covers all paths.
/// Called at the top of every mutating entry point.
#[inline(always)]
fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[contracttype]
pub enum MilestoneStatus {
    Pending = 0,
    Submitted = 1,
    Approved = 2,
    Rejected = 3,
}

#[derive(Clone)]
#[contracttype]
pub struct Milestone {
    pub title_hash: BytesN<32>,
    pub release_bps: u32,
    pub status: MilestoneStatus,
    pub evidence_hash: Option<BytesN<32>>,
}

#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    Creator,
    Platform,
    Escrow,
    Milestones,
    Initialized,
}

// Define the interface for the Escrow contract
#[contract]
pub struct MilestonesContract;

// Note: We use invoke_contract for cross-contract calls to avoid strict build dependencies
// on the escrow wasm during the initial build of this contract.
// Note: If the wasm is not available yet, we can use a manual client definition.
// Since we are building this together, I'll use a manual client to avoid build order issues.

#[contractimpl]
impl MilestonesContract {
    pub fn initialize(
        env: Env,
        creator: Address,
        platform: Address,
        escrow: Address,
        milestones: Vec<Milestone>,
    ) {
        if env.storage().instance().has(&DataKey::Initialized) {
            panic!("Already initialized");
        }

        let mut total_bps: u32 = 0;
        for m in milestones.iter() {
            total_bps += m.release_bps;
        }
        if total_bps != 10000 {
            panic!("Total BPS must be 10000");
        }

        env.storage().instance().set(&DataKey::Creator, &creator);
        env.storage().instance().set(&DataKey::Platform, &platform);
        env.storage().instance().set(&DataKey::Escrow, &escrow);
        env.storage().instance().set(&DataKey::Milestones, &milestones);
        env.storage().instance().set(&DataKey::Initialized, &true);

        // Stamp the initial TTL so the contract is live from the first ledger.
        bump_instance(&env);
    }

    pub fn submit_milestone(env: Env, index: u32, evidence_hash: BytesN<32>) {
        let creator: Address = env.storage().instance().get(&DataKey::Creator).expect("Not initialized");
        creator.require_auth();

        // Refresh instance TTL: a milestone submission is a material state
        // change that must be durably recorded for the platform review step.
        bump_instance(&env);

        let mut milestones: Vec<Milestone> = env.storage().instance().get(&DataKey::Milestones).expect("Not initialized");
        let mut milestone = milestones.get(index).expect("Invalid index");

        if milestone.status != MilestoneStatus::Pending && milestone.status != MilestoneStatus::Rejected {
            panic!("Milestone already submitted or approved");
        }

        milestone.status = MilestoneStatus::Submitted;
        milestone.evidence_hash = Some(evidence_hash.clone());
        milestones.set(index, milestone);
        env.storage().instance().set(&DataKey::Milestones, &milestones);

        env.events().publish(
            (symbol_short!("submit"), index),
            evidence_hash,
        );
    }

    pub fn approve_milestone(env: Env, index: u32) {
        let platform: Address = env.storage().instance().get(&DataKey::Platform).expect("Not initialized");
        platform.require_auth();

        // Refresh instance TTL: approval triggers a cross-contract fund release
        // — the most critical financial path in this contract.
        bump_instance(&env);

        let mut milestones: Vec<Milestone> = env.storage().instance().get(&DataKey::Milestones).expect("Not initialized");
        let mut milestone = milestones.get(index).expect("Invalid index");

        if milestone.status != MilestoneStatus::Submitted {
            panic!("Milestone not submitted");
        }

        milestone.status = MilestoneStatus::Approved;
        let release_bps = milestone.release_bps;
        milestones.set(index, milestone);
        env.storage().instance().set(&DataKey::Milestones, &milestones);

        let escrow_address: Address = env.storage().instance().get(&DataKey::Escrow).expect("Not initialized");
        let creator: Address = env.storage().instance().get(&DataKey::Creator).expect("Not initialized");

        // Use a cross-contract call to get the total raised amount from escrow
        let total_raised: i128 = env.invoke_contract(&escrow_address, &Symbol::new(&env, "get_total_raised"), Vec::new(&env));

        let release_amount = (total_raised * (release_bps as i128)) / 10000;

        if release_amount > 0 {
            // Approve the withdrawal in escrow (Milestones contract must be the Admin of Escrow)
            let _ : () = env.invoke_contract(&escrow_address, &Symbol::new(&env, "approve_withdrawal"), (release_amount,).into_val(&env));

            // Execute the withdrawal
            let _ : () = env.invoke_contract(&escrow_address, &Symbol::new(&env, "execute_withdrawal"), (creator.clone(), release_amount).into_val(&env));

            env.events().publish(
                (symbol_short!("release"), index),
                (creator, release_amount),
            );
        }

        env.events().publish(
            (symbol_short!("approve"), index),
            (),
        );
    }

    pub fn reject_milestone(env: Env, index: u32, reason_hash: BytesN<32>) {
        let platform: Address = env.storage().instance().get(&DataKey::Platform).expect("Not initialized");
        platform.require_auth();

        // Refresh instance TTL: rejection is a financial state change that
        // must persist so the creator can resubmit with corrected evidence.
        bump_instance(&env);

        let mut milestones: Vec<Milestone> = env.storage().instance().get(&DataKey::Milestones).expect("Not initialized");
        let mut milestone = milestones.get(index).expect("Invalid index");

        if milestone.status != MilestoneStatus::Submitted {
            panic!("Milestone not submitted");
        }

        milestone.status = MilestoneStatus::Rejected;
        milestones.set(index, milestone);
        env.storage().instance().set(&DataKey::Milestones, &milestones);

        env.events().publish(
            (symbol_short!("reject"), index),
            reason_hash,
        );
    }

    pub fn get_milestone(env: Env, index: u32) -> Milestone {
        let milestones: Vec<Milestone> = env.storage().instance().get(&DataKey::Milestones).expect("Not initialized");
        milestones.get(index).expect("Invalid index")
    }

    pub fn get_all_milestones(env: Env) -> Vec<Milestone> {
        env.storage().instance().get(&DataKey::Milestones).expect("Not initialized")
    }
}
