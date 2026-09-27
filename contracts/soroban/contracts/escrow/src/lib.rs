#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Env, Symbol};

/// Maximum platform fee the contract will ever accept, in basis points (10% = 1000 BPS).
///
/// This guards against configuration typos (e.g. `1000` instead of `100`, or `10000`
/// instead of `100`) that would otherwise route an unreasonable share — or all — of a
/// campaign's funds to the platform. Any fee above this cap is rejected outright.
pub const MAX_FEE_BPS: u32 = 1000;

// ---------------------------------------------------------------------------
// Storage TTL constants
// ---------------------------------------------------------------------------

/// Target TTL for instance storage entries (contract-global config, totals, etc.).
///
/// Set to ~120 days at 5 s/ledger  (120 × 24 × 3600 / 5 = 2_073_600 ledgers).
/// Every hot path refreshes instance TTL when the remaining lifetime drops below
/// `INSTANCE_TTL_THRESHOLD`, ensuring the contract remains alive throughout the
/// longest plausible campaign plus refund window.
pub const INSTANCE_TTL_EXTEND: u32 = 2_073_600;

/// If the current instance TTL falls at or below this value, extend it back to
/// `INSTANCE_TTL_EXTEND`.  Set to half the extend target so we never make an
/// unnecessary ledger round-trip on every single call.
pub const INSTANCE_TTL_THRESHOLD: u32 = 1_036_800;

/// Target TTL for persistent per-contributor balance entries.
///
/// Identical to the instance window: a contributor who deposited near the
/// campaign deadline must still be able to claim a refund 120 days later.
pub const PERSISTENT_TTL_EXTEND: u32 = 2_073_600;

/// Threshold below which a persistent entry is refreshed.  Half of the extend
/// target, consistent with the instance policy.
pub const PERSISTENT_TTL_THRESHOLD: u32 = 1_036_800;

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Refresh instance-storage TTL if it has fallen below the threshold.
///
/// Called at the top of every mutating entry point so the contract's config
/// and aggregate state cannot silently expire while a campaign is live.
#[inline(always)]
fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND);
}

/// Refresh a single persistent-storage entry's TTL if it has fallen below the
/// threshold.  Called whenever a contributor balance is read or written on a
/// hot financial path.
#[inline(always)]
fn bump_persistent(env: &Env, key: &DataKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, PERSISTENT_TTL_THRESHOLD, PERSISTENT_TTL_EXTEND);
}

#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    Admin,
    CampaignId,
    Target,
    Deadline,
    Asset,
    Balances(Address),
    TotalRaised,
    ApprovedWithdrawal,
    IsInitialized,
    PlatformFeeBps,
    PlatformFeeRecipient,
    PendingFeeBps,
}

#[contract]
pub struct EscrowContract;

#[contractimpl]
impl EscrowContract {
    pub fn initialize(
        env: Env,
        admin: Address,
        campaign_id: u64,
        target: i128,
        deadline: u64,
        asset: Address,
        platform_fee_bps: u32,
        platform_fee_recipient: Address,
    ) {
        if env.storage().instance().has(&DataKey::IsInitialized) {
            panic!("Contract is already initialized");
        }
        if platform_fee_bps > MAX_FEE_BPS {
            panic!("Platform fee BPS must not exceed MAX_FEE_BPS");
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::CampaignId, &campaign_id);
        env.storage().instance().set(&DataKey::Target, &target);
        env.storage().instance().set(&DataKey::Deadline, &deadline);
        env.storage().instance().set(&DataKey::Asset, &asset);
        env.storage().instance().set(&DataKey::TotalRaised, &0i128);
        env.storage().instance().set(&DataKey::ApprovedWithdrawal, &0i128);
        env.storage().instance().set(&DataKey::IsInitialized, &true);
        env.storage().instance().set(&DataKey::PlatformFeeBps, &platform_fee_bps);
        env.storage().instance().set(&DataKey::PlatformFeeRecipient, &platform_fee_recipient);

        // Stamp the initial TTL so the contract is live from the first ledger.
        bump_instance(&env);
    }

    pub fn deposit(env: Env, from: Address, amount: i128) {
        from.require_auth();

        // Refresh instance TTL before any state read so config cannot expire
        // mid-campaign while deposits are still flowing.
        bump_instance(&env);

        let deadline: u64 = env.storage().instance().get(&DataKey::Deadline).unwrap();
        let current_time = env.ledger().timestamp();
        if current_time > deadline {
            panic!("Campaign deadline has passed");
        }

        let asset: Address = env.storage().instance().get(&DataKey::Asset).unwrap();
        let client = token::Client::new(&env, &asset);
        client.transfer(&from, &env.current_contract_address(), &amount);

        let balance_key = DataKey::Balances(from.clone());
        let current_balance: i128 = env.storage().persistent().get(&balance_key).unwrap_or(0);
        let new_balance = current_balance + amount;
        env.storage().persistent().set(&balance_key, &new_balance);
        // Keep the contributor's balance entry alive at least as long as the
        // refund window (deadline + full persistent TTL window).
        bump_persistent(&env, &balance_key);

        let total_raised: i128 = env.storage().instance().get(&DataKey::TotalRaised).unwrap_or(0);
        env.storage().instance().set(&DataKey::TotalRaised, &(total_raised + amount));

        env.events().publish(
            (Symbol::new(&env, "deposit"), from),
            amount,
        );
    }

    pub fn approve_withdrawal(env: Env, release_amount: i128) {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();

        bump_instance(&env);

        let approved: i128 = env.storage().instance().get(&DataKey::ApprovedWithdrawal).unwrap_or(0);
        env.storage().instance().set(&DataKey::ApprovedWithdrawal, &(approved + release_amount));
    }

    /// Step 1 of 2: the admin proposes a new platform fee.
    ///
    /// The proposal is validated against [`MAX_FEE_BPS`] and stored as pending, but it
    /// does **not** take effect until [`Self::confirm_fee_change`] is called in a separate
    /// transaction. Requiring two explicit admin actions makes an accidental, fund-draining
    /// fee change far harder to trigger by a single typo.
    pub fn propose_fee_change(env: Env, new_fee_bps: u32) {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();

        if new_fee_bps > MAX_FEE_BPS {
            panic!("Platform fee BPS must not exceed MAX_FEE_BPS");
        }

        bump_instance(&env);

        env.storage().instance().set(&DataKey::PendingFeeBps, &new_fee_bps);

        env.events().publish(
            (Symbol::new(&env, "fee_proposed"), admin),
            new_fee_bps,
        );
    }

    /// Step 2 of 2: the admin confirms the pending fee, applying it.
    ///
    /// Panics if there is no pending proposal. The pending value is re-validated against
    /// [`MAX_FEE_BPS`] as a defense-in-depth measure before it becomes the active fee.
    pub fn confirm_fee_change(env: Env) {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();

        let new_fee_bps: u32 = match env.storage().instance().get(&DataKey::PendingFeeBps) {
            Some(bps) => bps,
            None => panic!("No pending fee change to confirm"),
        };

        if new_fee_bps > MAX_FEE_BPS {
            panic!("Platform fee BPS must not exceed MAX_FEE_BPS");
        }

        bump_instance(&env);

        env.storage().instance().set(&DataKey::PlatformFeeBps, &new_fee_bps);
        env.storage().instance().remove(&DataKey::PendingFeeBps);

        env.events().publish(
            (Symbol::new(&env, "fee_changed"), admin),
            new_fee_bps,
        );
    }

    /// Cancels any pending fee proposal without applying it.
    pub fn cancel_fee_change(env: Env) {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();

        if !env.storage().instance().has(&DataKey::PendingFeeBps) {
            panic!("No pending fee change to cancel");
        }

        bump_instance(&env);

        env.storage().instance().remove(&DataKey::PendingFeeBps);

        env.events().publish(
            (Symbol::new(&env, "fee_cancelled"), admin),
            (),
        );
    }

    /// Returns the pending proposed fee in BPS, or `None` if no change is pending.
    pub fn get_pending_fee(env: Env) -> Option<u32> {
        env.storage().instance().get(&DataKey::PendingFeeBps)
    }

    pub fn execute_withdrawal(env: Env, to: Address, release_amount: i128) {
        bump_instance(&env);

        let mut approved: i128 = env.storage().instance().get(&DataKey::ApprovedWithdrawal).unwrap_or(0);
        if approved < release_amount {
            panic!("Insufficient approved amount");
        }

        let asset: Address = env.storage().instance().get(&DataKey::Asset).unwrap();
        let client = token::Client::new(&env, &asset);

        let fee_bps: u32 = env.storage().instance().get(&DataKey::PlatformFeeBps).unwrap_or(0);
        let fee_amount = (release_amount * (fee_bps as i128)) / 10000;
        let net_amount = release_amount - fee_amount;

        if net_amount > 0 {
            client.transfer(&env.current_contract_address(), &to, &net_amount);
        }

        if fee_amount > 0 {
            let fee_recipient: Address = env.storage().instance().get(&DataKey::PlatformFeeRecipient).unwrap();
            client.transfer(&env.current_contract_address(), &fee_recipient, &fee_amount);
        }

        approved -= release_amount;
        env.storage().instance().set(&DataKey::ApprovedWithdrawal, &approved);

        env.events().publish(
            (Symbol::new(&env, "withdrawal"), to),
            (release_amount, net_amount, fee_amount),
        );
    }

    pub fn refund(env: Env, contributor: Address) {
        bump_instance(&env);

        let deadline: u64 = env.storage().instance().get(&DataKey::Deadline).unwrap();
        if env.ledger().timestamp() < deadline {
            panic!("Deadline has not passed");
        }

        let total_raised: i128 = env.storage().instance().get(&DataKey::TotalRaised).unwrap_or(0);
        let target: i128 = env.storage().instance().get(&DataKey::Target).unwrap();

        if total_raised >= target {
            panic!("Campaign succeeded, refunds unavailable");
        }

        let balance_key = DataKey::Balances(contributor.clone());

        // Extend the persistent entry TTL before reading so the lookup cannot
        // fail with an expiry-induced miss even if many ledgers have elapsed.
        bump_persistent(&env, &balance_key);

        let amount: i128 = env.storage().persistent().get(&balance_key).unwrap_or(0);
        if amount <= 0 {
            panic!("No contribution to refund");
        }

        let asset: Address = env.storage().instance().get(&DataKey::Asset).unwrap();
        let client = token::Client::new(&env, &asset);
        client.transfer(&env.current_contract_address(), &contributor, &amount);

        env.storage().persistent().set(&balance_key, &0i128);
        // No need to re-extend after zeroing — the entry will expire naturally.

        let total_raised_current: i128 = env.storage().instance().get(&DataKey::TotalRaised).unwrap_or(0);
        let new_total = total_raised_current - amount;
        env.storage().instance().set(&DataKey::TotalRaised, &new_total);

        env.events().publish(
            (Symbol::new(&env, "refund"), contributor),
            amount,
        );
    }

    pub fn get_total_raised(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::TotalRaised).unwrap_or(0)
    }

    pub fn get_asset(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Asset).unwrap()
    }

    pub fn get_platform_fee_config(env: Env) -> (u32, Address) {
        let bps: u32 = env.storage().instance().get(&DataKey::PlatformFeeBps).unwrap_or(0);
        let recipient: Address = env.storage().instance().get(&DataKey::PlatformFeeRecipient).unwrap();
        (bps, recipient)
    }
}
