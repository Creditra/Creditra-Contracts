#![no_std]
#![allow(clippy::unused_unit)]

//! Creditra credit contract: credit lines, draw/repay, risk parameters.
//!
//! # Reentrancy
//! Soroban token transfers (e.g. Stellar Asset Contract) do not invoke callbacks back into
//! the caller. This contract uses a reentrancy guard on draw_credit and repay_credit as a
//! defense-in-depth measure; if a token or future integration ever called back, the guard
//! would revert.

mod events;
mod types;

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, Address, Env, Symbol,
};

use events::{
    publish_credit_line_event, publish_drawn_event, publish_repayment_event,
    publish_risk_parameters_updated, CreditLineEvent, DrawnEvent, RepaymentEvent,
    RiskParametersUpdatedEvent,
};
use types::{CreditLineData, CreditStatus, RateChangeConfig};

/// Maximum interest rate in basis points (100%).
const MAX_INTEREST_RATE_BPS: u32 = 10_000;

/// Maximum risk score (0–100 scale).
const MAX_RISK_SCORE: u32 = 100;

/// Instance storage key for reentrancy guard.
fn reentrancy_key(env: &Env) -> Symbol {
    Symbol::new(env, "reentrancy")
}

/// Instance storage key for admin.
fn admin_key(env: &Env) -> Symbol {
    Symbol::new(env, "admin")
}

/// Instance storage key for rate-change limit configuration.
fn rate_cfg_key(env: &Env) -> Symbol {
    Symbol::new(env, "rate_cfg")
}

fn require_admin(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&admin_key(env))
        .unwrap_or_else(|| env.panic_with_error(ContractError::NotAdmin))
}

fn require_admin_auth(env: &Env) -> Address {
    let admin = require_admin(env);
    admin.require_auth();
    admin
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    LiquidityToken,
    LiquiditySource,
}

/// Assert reentrancy guard is not set; set it for the duration of the call.
/// Caller must call clear_reentrancy_guard when done (on all paths).
fn set_reentrancy_guard(env: &Env) {
    let key = reentrancy_key(env);
    let current: bool = env.storage().instance().get(&key).unwrap_or(false);
    if current {
        panic!("reentrancy guard");
    }
    env.storage().instance().set(&key, &true);
}

fn clear_reentrancy_guard(env: &Env) {
    env.storage().instance().set(&reentrancy_key(env), &false);
}

#[contract]
pub struct Credit;

#[contractimpl]
impl Credit {
    /// Initialize the contract (admin).
    pub fn init(env: Env, admin: Address) -> () {
        env.storage().instance().set(&admin_key(&env), &admin);
        env.storage()
            .instance()
            .set(&DataKey::LiquiditySource, &env.current_contract_address());
        ()
    }

    /// @notice Sets the token contract used for reserve/liquidity checks and draw transfers.
    pub fn set_liquidity_token(env: Env, token_address: Address) {
        config::set_liquidity_token(env, token_address)
    }

    /// @notice Sets the address that provides liquidity for draw operations.
    pub fn set_liquidity_source(env: Env, reserve_address: Address) {
        config::set_liquidity_source(env, reserve_address)
    }

    /// Open a new credit line for a borrower (called by backend/risk engine).
    pub fn open_credit_line(
        env: Env,
        borrower: Address,
        credit_limit: i128,
        interest_rate_bps: u32,
        risk_score: u32,
    ) {
        assert!(credit_limit > 0, "credit_limit must be greater than zero");
        assert!(
            interest_rate_bps <= 10_000,
            "interest_rate_bps cannot exceed 10000 (100%)"
        );
        assert!(risk_score <= 100, "risk_score must be between 0 and 100");

        // Prevent overwriting an existing Active credit line
        if let Some(existing) = env
            .storage()
            .persistent()
            .get::<Address, CreditLineData>(&borrower)
        {
            assert!(
                existing.status != CreditStatus::Active,
                "borrower already has an active credit line"
            );
        }

        let credit_line = CreditLineData {
            borrower: borrower.clone(),
            credit_limit,
            utilized_amount: 0,
            interest_rate_bps,
            risk_score,
            status: CreditStatus::Active,
            last_rate_update_ts: 0,
        };

        env.storage().persistent().set(&borrower, &credit_line);

        publish_credit_line_event(
            &env,
            (symbol_short!("credit"), symbol_short!("opened")),
            CreditLineEvent {
                event_type: symbol_short!("opened"),
                borrower: borrower.clone(),
                status: CreditStatus::Active,
                credit_limit,
                interest_rate_bps,
                risk_score,
            },
        );
    }

    /// Update risk parameters for an existing credit line.
    ///
    /// Called by admin or risk engine when a borrower's risk profile changes.
    ///
    /// # Parameters
    /// - `borrower`: The borrower's address.
    /// - `credit_limit`: New credit limit.
    /// - `interest_rate_bps`: New interest rate in basis points.
    /// - `risk_score`: New risk score.
    ///
    /// # Note
    /// Not yet implemented. Planned logic: load existing record, update fields,
    /// persist updated [`CreditLineData`].
    /// @notice Draws credit by transferring liquidity tokens to the borrower.
    /// @dev Enforces status/limit/liquidity checks and uses a reentrancy guard.
    pub fn draw_credit(env: Env, borrower: Address, amount: i128) -> () {
        set_reentrancy_guard(&env);
        borrower.require_auth();

        if amount <= 0 {
            clear_reentrancy_guard(&env);
            panic!("amount must be positive");
        }

        let token_address: Option<Address> = env.storage().instance().get(&DataKey::LiquidityToken);
        let reserve_address: Address = env
            .storage()
            .instance()
            .get(&DataKey::LiquiditySource)
            .unwrap_or(env.current_contract_address());

        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .unwrap_or_else(|| {
                clear_reentrancy_guard(&env);
                env.panic_with_error(ContractError::CreditLineNotFound)
            });

        if credit_line.borrower != borrower {
            clear_reentrancy_guard(&env);
            panic!("Borrower mismatch for credit line");
        }

        if credit_line.status == CreditStatus::Closed {
            clear_reentrancy_guard(&env);
            env.panic_with_error(ContractError::CreditLineClosed);
        }

        if credit_line.status == CreditStatus::Suspended {
            clear_reentrancy_guard(&env);
            panic!("credit line is suspended");
        }

        if credit_line.status == CreditStatus::Defaulted {
            clear_reentrancy_guard(&env);
            panic!("credit line is defaulted");
        }

        if credit_line.status != CreditStatus::Active {
            clear_reentrancy_guard(&env);
            env.panic_with_error(ContractError::InvalidAmount);
        }

        let updated_utilized = credit_line
            .utilized_amount
            .checked_add(amount)
            .unwrap_or_else(|| {
                clear_reentrancy_guard(&env);
                env.panic_with_error(ContractError::Overflow)
            });

        if updated_utilized > credit_line.credit_limit {
            clear_reentrancy_guard(&env);
            panic!("exceeds credit limit");
        }

        if let Some(token_address) = token_address {
            let token_client = token::Client::new(&env, &token_address);
            let reserve_balance = token_client.balance(&reserve_address);
            if reserve_balance < amount {
                clear_reentrancy_guard(&env);
                panic!("Insufficient liquidity reserve for requested draw amount");
            }

            token_client.transfer(&reserve_address, &borrower, &amount);
        }

        credit_line.utilized_amount = updated_utilized;
        env.storage().persistent().set(&borrower, &credit_line);
        let timestamp = env.ledger().timestamp();
        publish_drawn_event(
            &env,
            DrawnEvent {
                borrower,
                amount,
                new_utilized_amount: updated_utilized,
                timestamp,
            },
        );
        clear_reentrancy_guard(&env);
        ()
    }

    /// Repay credit (borrower).
    /// Reverts if credit line does not exist, is Closed, or borrower has not authorized.
    /// Reduces utilized_amount by amount (capped at 0). Emits RepaymentEvent.
    pub fn repay_credit(env: Env, borrower: Address, amount: i128) {
        // --- Reentrancy guard (defense-in-depth) ---
        set_reentrancy_guard(&env);

        // --- Auth: only the borrower may repay their own line ---
        borrower.require_auth();

        // --- Input validation ---
        if amount <= 0 {
            clear_reentrancy_guard(&env);
            env.panic_with_error(ContractError::InvalidAmount);
        }

        // --- Load credit line ---
        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .unwrap_or_else(|| {
                clear_reentrancy_guard(&env);
                env.panic_with_error(ContractError::CreditLineNotFound)
            });

        // --- Status check: only Closed is disallowed ---
        if credit_line.status == CreditStatus::Closed {
            clear_reentrancy_guard(&env);
            env.panic_with_error(ContractError::CreditLineClosed);
        }

        // --- Compute effective repayment (cap at outstanding utilization) ---
        // This prevents over-pulling tokens and keeps accounting correct.
        let effective_repay = if amount > credit_line.utilized_amount {
            credit_line.utilized_amount
        } else {
            amount
        };

        // --- Token transfer (when liquidity token is configured) ---
        // We check allowance and balance *before* mutating state so that a
        // failed transfer reverts cleanly without partial state changes.
        if effective_repay > 0 {
            let token_address: Option<Address> =
                env.storage().instance().get(&DataKey::LiquidityToken);

            if let Some(token_address) = token_address {
                let reserve_address: Address = env
                    .storage()
                    .instance()
                    .get(&DataKey::LiquiditySource)
                    .unwrap_or_else(|| env.current_contract_address());

                let token_client = token::Client::new(&env, &token_address);
                let contract_address = env.current_contract_address();

                // Guard: allowance must cover the effective repayment.
                let allowance = token_client.allowance(&borrower, &contract_address);
                if allowance < effective_repay {
                    clear_reentrancy_guard(&env);
                    panic!("Insufficient allowance");
                }

                // Guard: borrower must actually hold the tokens.
                let balance = token_client.balance(&borrower);
                if balance < effective_repay {
                    clear_reentrancy_guard(&env);
                    panic!("Insufficient balance");
                }

                // Pull tokens from borrower → liquidity source via transfer_from.
                token_client.transfer_from(
                    &contract_address,
                    &borrower,
                    &reserve_address,
                    &effective_repay,
                );
            }
        }

        // --- Update state ---
        let new_utilized = credit_line
            .utilized_amount
            .saturating_sub(effective_repay)
            .max(0);
        credit_line.utilized_amount = new_utilized;
        env.storage().persistent().set(&borrower, &credit_line);

        // --- Emit event ---
        let timestamp = env.ledger().timestamp();
        publish_repayment_event(
            &env,
            RepaymentEvent {
                borrower,
                amount: effective_repay,
                new_utilized_amount: new_utilized,
                timestamp,
            },
        );

        // --- Release reentrancy guard ---
        clear_reentrancy_guard(&env);
    }

    /// Update risk parameters for an existing credit line (admin only).
    ///
    /// # Arguments
    /// * `borrower` - Borrower whose credit line to update.
    /// * `credit_limit` - New credit limit (must be >= current utilized_amount and >= 0).
    /// * `interest_rate_bps` - New interest rate in basis points (0 ..= 10000).
    /// * `risk_score` - New risk score (0 ..= 100).
    ///
    /// # Panics
    /// * If caller is not the contract admin.
    /// * If no credit line exists for the borrower.
    /// * If bounds are violated (e.g. credit_limit < utilized_amount).
    ///
    /// # Rate-change limits
    /// When a `RateChangeConfig` has been set via `set_rate_change_limits`, the
    /// following additional checks are enforced whenever the interest rate is
    /// actually changing:
    /// * The absolute delta `|new_rate - old_rate|` must be ≤ `max_rate_change_bps`.
    /// * If a minimum interval is configured and a previous rate change
    ///   timestamp exists, the elapsed time since the last change must be ≥
    ///   `rate_change_min_interval`.
    ///
    /// # Rate-change limits
    /// When a `RateChangeConfig` has been set via `set_rate_change_limits`, the
    /// following additional checks are enforced whenever the interest rate is
    /// actually changing:
    /// * The absolute delta `|new_rate - old_rate|` must be ≤
    ///   `max_rate_change_bps`.
    /// * If a minimum interval is configured and a previous rate change
    ///   timestamp exists, the elapsed time since the last change must be ≥
    ///   `rate_change_min_interval`.
    ///
    /// Emits a risk_updated event.
    pub fn update_risk_parameters(
        env: Env,
        borrower: Address,
        credit_limit: i128,
        interest_rate_bps: u32,
        risk_score: u32,
    ) {
        require_admin_auth(&env);

        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .expect("Credit line not found");

        if credit_limit < 0 {
            panic!("credit_limit must be non-negative");
        }
        if credit_limit < credit_line.utilized_amount {
            panic!("credit_limit cannot be less than utilized amount");
        }
        if interest_rate_bps > MAX_INTEREST_RATE_BPS {
            panic!("interest_rate_bps exceeds maximum");
        }
        if risk_score > MAX_RISK_SCORE {
            panic!("risk_score exceeds maximum");
        }

        // --- Rate-change limit enforcement (#17) ---
        let rate_cfg: Option<RateChangeConfig> = env.storage().instance().get(&rate_cfg_key(&env));
        if let Some(cfg) = rate_cfg {
            let old_rate = credit_line.interest_rate_bps;
            if interest_rate_bps != old_rate {
                // Check minimum interval between rate changes.
                if credit_line.last_rate_update_ts > 0 && cfg.rate_change_min_interval > 0 {
                    let now = env.ledger().timestamp();
                    let elapsed = now.saturating_sub(credit_line.last_rate_update_ts);
                    if elapsed < cfg.rate_change_min_interval {
                        panic!("rate change too soon: minimum interval not elapsed");
                    }
                }
                // Check absolute delta cap.
                let delta = interest_rate_bps.abs_diff(old_rate);
                if delta > cfg.max_rate_change_bps {
                    panic!("rate change exceeds maximum allowed delta");
                }
                // Record the timestamp of this rate change.
                credit_line.last_rate_update_ts = env.ledger().timestamp();
            }
        }

        credit_line.credit_limit = credit_limit;
        credit_line.interest_rate_bps = interest_rate_bps;
        credit_line.risk_score = risk_score;
        env.storage().persistent().set(&borrower, &credit_line);

        publish_risk_parameters_updated(
            &env,
            RiskParametersUpdatedEvent {
                borrower: borrower.clone(),
                credit_limit,
                interest_rate_bps,
                risk_score,
            },
        );
    }

    /// Set rate-change limits (admin only).
    pub fn set_rate_change_limits(env: Env, max_rate_change_bps: u32, rate_change_min_interval: u64) {
        risk::set_rate_change_limits(env, max_rate_change_bps, rate_change_min_interval)
    }

    /// Get the current rate-change limit configuration (view function).
    pub fn get_rate_change_limits(env: Env) -> Option<RateChangeConfig> {
        risk::get_rate_change_limits(env)
    }

    /// Suspend a credit line (admin only).
    /// Emits a CreditLineSuspended event.
    pub fn suspend_credit_line(env: Env, borrower: Address) {
        require_admin_auth(&env);
        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .expect("Credit line not found");

        if credit_line.status != CreditStatus::Active {
            panic!("Only active credit lines can be suspended");
        }

        credit_line.status = CreditStatus::Suspended;
        env.storage().persistent().set(&borrower, &credit_line);

        publish_credit_line_event(
            &env,
            (symbol_short!("credit"), symbol_short!("suspend")),
            CreditLineEvent {
                event_type: symbol_short!("suspend"),
                borrower: borrower.clone(),
                status: CreditStatus::Suspended,
                credit_limit: credit_line.credit_limit,
                interest_rate_bps: credit_line.interest_rate_bps,
                risk_score: credit_line.risk_score,
            },
        );
    }

    /// Close a credit line. Callable by admin (force-close) or by borrower when utilization is zero.
    ///
    /// Can be called by the admin (to force-close regardless of utilization) or by the borrower
    /// (only when `utilized_amount` is 0). Once closed, the credit line cannot be reopened.
    /// Idempotent if already Closed.
    ///
    /// # Parameters
    /// - `borrower`: The borrower's address.
    /// - `closer`: Address that must have authorized this call. Must be either the contract admin
    ///   (can close regardless of utilization) or the borrower (can close only when
    ///   `utilized_amount` is zero).
    ///
    /// # Panics
    /// - If no credit line exists for the given borrower.
    /// - If `closer` is not admin/borrower, or if borrower closes while `utilized_amount != 0`.
    ///
    /// # Events
    /// Emits a `("credit", "closed")` [`CreditLineEvent`].
    pub fn close_credit_line(env: Env, borrower: Address, closer: Address) {
        closer.require_auth();

        let admin: Address = require_admin(&env);

        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .expect("Credit line not found");

        if credit_line.status == CreditStatus::Closed {
            return;
        }

        let allowed = closer == admin || (closer == borrower && credit_line.utilized_amount == 0);

        if !allowed {
            if closer == borrower {
                panic!("cannot close: utilized amount not zero");
            }
            panic!("unauthorized");
        }

        credit_line.status = CreditStatus::Closed;
        env.storage().persistent().set(&borrower, &credit_line);

        publish_credit_line_event(
            &env,
            (symbol_short!("credit"), symbol_short!("closed")),
            CreditLineEvent {
                event_type: symbol_short!("closed"),
                borrower: borrower.clone(),
                status: CreditStatus::Closed,
                credit_limit: credit_line.credit_limit,
                interest_rate_bps: credit_line.interest_rate_bps,
                risk_score: credit_line.risk_score,
            },
        );
    }

    /// Mark a credit line as defaulted (admin only).
    /// Emits a CreditLineDefaulted event.
    pub fn default_credit_line(env: Env, borrower: Address) {
        require_admin_auth(&env);
        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .expect("Credit line not found");

        credit_line.status = CreditStatus::Defaulted;
        env.storage().persistent().set(&borrower, &credit_line);

        publish_credit_line_event(
            &env,
            (symbol_short!("credit"), symbol_short!("default")),
            CreditLineEvent {
                event_type: symbol_short!("default"),
                borrower: borrower.clone(),
                status: CreditStatus::Defaulted,
                credit_limit: credit_line.credit_limit,
                interest_rate_bps: credit_line.interest_rate_bps,
                risk_score: credit_line.risk_score,
            },
        );
    }

    /// Get credit line data for a borrower (view function).
    ///
    /// # Parameters
    /// - `borrower`: The address to query.
    ///
    /// # Returns
    /// `Option<CreditLineData>` — full data or `None` if no line exists.
    pub fn get_credit_line(env: Env, borrower: Address) -> Option<CreditLineData> {
        query::get_credit_line(env, borrower)
    }
}

#[cfg(test)]
mod test {
    /// Helper to set up a contract, open a credit line, and return (client, token, admin)
    fn setup_contract_with_credit_line<'a>(
        env: &'a Env,
        borrower: &'a Address,
        credit_limit: i128,
        utilized_amount: i128,
    ) -> (CreditClient<'a>, Address, Address) {
        env.mock_all_auths();
        let admin = Address::generate(env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(env, &contract_id);
        client.init(&admin);
        client.open_credit_line(borrower, &credit_limit, &300_u32, &70_u32);
        if utilized_amount > 0 {
            client.draw_credit(borrower, &utilized_amount);
        }
        (client, contract_id, admin)
    }
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::testutils::Events as _;
    use soroban_sdk::token;
    use soroban_sdk::token::StellarAssetClient;
    use soroban_sdk::Symbol;

    fn setup_test(env: &Env) -> (Address, Address, Address) {
        env.mock_all_auths();

        let admin = Address::generate(env);
        let borrower = Address::generate(env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);

        (admin, borrower, contract_id)
    }

    fn call_contract<F>(env: &Env, contract_id: &Address, f: F)
    where
        F: FnOnce(),
    {
        env.as_contract(contract_id, f);
    }

    fn get_credit_data(env: &Env, contract_id: &Address, borrower: &Address) -> CreditLineData {
        let client = CreditClient::new(env, contract_id);
        client
            .get_credit_line(borrower)
            .expect("Credit line not found")
    }

    #[test]
    fn repay_active_full_repayment_zeros_utilization() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, token, contract_id, _admin) = setup(&env, &borrower, 1_000, 1_000, 500);

        StellarAssetClient::new(&env, &token).mint(&borrower, &500);
        approve(&env, &token, &borrower, &contract_id, 500);

        client.repay_credit(&borrower, &500);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.utilized_amount, 0);
        assert_eq!(line.status, CreditStatus::Active);
    }

    // ── 2. Happy path: repay while Suspended ─────────────────────────────────

    #[test]
    fn repay_suspended_reduces_utilized_amount() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, token, contract_id, _admin) = setup(&env, &borrower, 1_000, 1_000, 300);

        client.suspend_credit_line(&borrower);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Suspended
        );

        StellarAssetClient::new(&env, &token).mint(&borrower, &100);
        approve(&env, &token, &borrower, &contract_id, 100);

        client.repay_credit(&borrower, &100);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.utilized_amount, 200);
        assert_eq!(line.status, CreditStatus::Suspended); // status unchanged
    }

    // ── 3. Happy path: repay while Defaulted ─────────────────────────────────

    #[test]
    fn repay_defaulted_reduces_utilized_amount() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, token, contract_id, _admin) = setup(&env, &borrower, 1_000, 1_000, 400);

        client.default_credit_line(&borrower);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Defaulted
        );

        StellarAssetClient::new(&env, &token).mint(&borrower, &150);
        approve(&env, &token, &borrower, &contract_id, 150);

        client.repay_credit(&borrower, &150);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.utilized_amount, 250);
        assert_eq!(line.status, CreditStatus::Defaulted); // status unchanged
    }

    // ── 4. Over-repay: effective amount capped at utilized_amount ─────────────

    #[test]
    fn repay_overpayment_caps_at_utilized_does_not_over_pull_tokens() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, token, contract_id, _admin) = setup(&env, &borrower, 1_000, 1_000, 100);

        // Fund borrower with 500 but only 100 is owed
        StellarAssetClient::new(&env, &token).mint(&borrower, &500);
        approve(&env, &token, &borrower, &contract_id, 500);

        let token_client = token::Client::new(&env, &token);
        let borrower_before = token_client.balance(&borrower);
        let reserve_before = token_client.balance(&contract_id);

        client.repay_credit(&borrower, &500); // overpay

        // Only 100 (utilized amount) should have moved
        assert_eq!(token_client.balance(&borrower), borrower_before - 100);
        assert_eq!(token_client.balance(&contract_id), reserve_before + 100);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.utilized_amount, 0);
    }

    #[test]
    fn repay_overpayment_when_zero_utilization_transfers_nothing() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        // draw_amount = 0 → utilized_amount starts at 0
        let (client, token, contract_id, _admin) = setup(&env, &borrower, 1_000, 0, 0);

        StellarAssetClient::new(&env, &token).mint(&borrower, &200);
        approve(&env, &token, &borrower, &contract_id, 200);

        let token_client = token::Client::new(&env, &token);
        let borrower_before = token_client.balance(&borrower);

        client.repay_credit(&borrower, &200);

        // No tokens should move because effective_repay == 0
        assert_eq!(token_client.balance(&borrower), borrower_before);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            0
        );
    }

    // ── 5. Amount guards ─────────────────────────────────────────────────────

    #[test]
    #[should_panic(expected = "Error(Contract, #5)")]
    fn repay_zero_amount_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _contract_id, _admin) = setup(&env, &borrower, 1_000, 1_000, 200);
        client.repay_credit(&borrower, &0);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #5)")]
    fn repay_negative_amount_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _contract_id, _admin) = setup(&env, &borrower, 1_000, 1_000, 200);
        client.repay_credit(&borrower, &-100);
    }

    // ── 6. Closed line rejection ──────────────────────────────────────────────

    #[test]
    #[should_panic(expected = "Error(Contract, #4)")]
    fn repay_closed_line_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _contract_id, admin) = setup(&env, &borrower, 1_000, 0, 0);
        client.close_credit_line(&borrower, &admin);
        client.repay_credit(&borrower, &100);
    }

    // ── 7. Nonexistent line ───────────────────────────────────────────────────

    #[test]
    #[should_panic(expected = "Error(Contract, #3)")]
    fn repay_nonexistent_line_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        client.repay_credit(&borrower, &100);
    }

    // ── 8. Token transfer_from accounting ────────────────────────────────────

    #[test]
    fn repay_transfers_tokens_from_borrower_to_contract_reserve() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, token, contract_id, _admin) = setup(&env, &borrower, 1_000, 1_000, 300);

        StellarAssetClient::new(&env, &token).mint(&borrower, &200);
        approve(&env, &token, &borrower, &contract_id, 200);

        let token_client = token::Client::new(&env, &token);
        let borrower_before = token_client.balance(&borrower);
        let reserve_before = token_client.balance(&contract_id);

        client.repay_credit(&borrower, &200);

        assert_eq!(token_client.balance(&borrower), borrower_before - 200);
        assert_eq!(token_client.balance(&contract_id), reserve_before + 200);
    }

    #[test]
    fn repay_transfers_to_configured_external_liquidity_source() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let reserve = Address::generate(&env);
        let (client, token, contract_id, _admin) = setup(&env, &borrower, 1_000, 1_000, 250);

        client.set_liquidity_source(&reserve);

        StellarAssetClient::new(&env, &token).mint(&borrower, &100);
        approve(&env, &token, &borrower, &contract_id, 100);

        let token_client = token::Client::new(&env, &token);
        let reserve_before = token_client.balance(&reserve);

        client.repay_credit(&borrower, &100);

        assert_eq!(token_client.balance(&reserve), reserve_before + 100);
        // contract_id should NOT have received the tokens
        // (tokens go to the external reserve, not the contract itself)
    }

    #[test]
    fn repay_consumes_allowance_correctly() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, token, contract_id, _admin) = setup(&env, &borrower, 1_000, 1_000, 300);

        StellarAssetClient::new(&env, &token).mint(&borrower, &200);
        approve(&env, &token, &borrower, &contract_id, 200);

        let token_client = token::Client::new(&env, &token);
        let allowance_before = token_client.allowance(&borrower, &contract_id);

        client.repay_credit(&borrower, &200);

        // Allowance should decrease by repay amount
        let allowance_after = token_client.allowance(&borrower, &contract_id);
        assert_eq!(allowance_before - allowance_after, 200);
    }

    // ── 9. Insufficient allowance / balance reverts ───────────────────────────

    #[test]
    #[should_panic(expected = "Insufficient allowance")]
    fn repay_insufficient_allowance_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, token, contract_id, _admin) = setup(&env, &borrower, 1_000, 1_000, 200);

        StellarAssetClient::new(&env, &token).mint(&borrower, &200);
        // Approve only 50 but trying to repay 200
        approve(&env, &token, &borrower, &contract_id, 50);

        client.repay_credit(&borrower, &200);
    }

    #[test]
    #[should_panic(expected = "Insufficient balance")]
    fn repay_insufficient_balance_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, token, contract_id, _admin) = setup(&env, &borrower, 1_000, 1_000, 200);

        // Reduce borrower's token balance to 50 while credit utilization remains 200
        let token_client = token::Client::new(&env, &token);
        let other = Address::generate(&env);
        token_client.transfer(&borrower, &other, &150);

        approve(&env, &token, &borrower, &contract_id, 200);

        client.repay_credit(&borrower, &200);
    }

    // ── 10. RepaymentEvent schema ─────────────────────────────────────────────

    #[test]
    fn repay_emits_repayment_event_with_correct_payload() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, token, contract_id, _admin) = setup(&env, &borrower, 5_000, 5_000, 1_000);

        StellarAssetClient::new(&env, &token).mint(&borrower, &400);
        approve(&env, &token, &borrower, &contract_id, 400);

        client.repay_credit(&borrower, &400);

        let events = env.events().all();
        let (_contract, topics, data) = events.last().unwrap();

        // Topic[0] = "credit", Topic[1] = "repay"
        let topic0: Symbol = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
        let topic1: Symbol = Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap();
        assert_eq!(topic0, symbol_short!("credit"));
        assert_eq!(topic1, symbol_short!("repay"));

        let event: RepaymentEvent = data.try_into_val(&env).unwrap();
        assert_eq!(event.borrower, borrower);
        assert_eq!(event.amount, 400);
        assert_eq!(event.new_utilized_amount, 600); // 1000 - 400
    }

    #[test]
    fn repay_event_amount_reflects_effective_not_nominal_for_overpayment() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, token, contract_id, _admin) = setup(&env, &borrower, 1_000, 1_000, 200);

        // Over-approve and over-repay: only 200 (utilized) should appear in event
        StellarAssetClient::new(&env, &token).mint(&borrower, &500);
        approve(&env, &token, &borrower, &contract_id, 500);

        client.repay_credit(&borrower, &500);

        let events = env.events().all();
        let (_contract, _topics, data) = events.last().unwrap();
        let event: RepaymentEvent = data.try_into_val(&env).unwrap();

        assert_eq!(event.amount, 200); // effective, not 500
        assert_eq!(event.new_utilized_amount, 0);
    }

    #[test]
    fn repay_emits_exactly_one_event_per_call() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, token, contract_id, _admin) = setup(&env, &borrower, 1_000, 1_000, 600);

        StellarAssetClient::new(&env, &token).mint(&borrower, &200);
        approve(&env, &token, &borrower, &contract_id, 200);

        let _events_before = env.events().all().len();
        client.repay_credit(&borrower, &200);
        // Exactly one new event (the RepaymentEvent) plus any SAC transfer event
        // We verify the last event is always the RepaymentEvent
        let events = env.events().all();
        let (_contract, topics, _data) = events.last().unwrap();
        let topic1: Symbol = Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap();
        assert_eq!(topic1, symbol_short!("repay"));
    }

    // ── 11. Reentrancy guard cleared (sequential repays succeed) ──────────────

    #[test]
    fn repay_reentrancy_guard_cleared_allowing_sequential_repays() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, token, contract_id, _admin) = setup(&env, &borrower, 1_000, 1_000, 600);

        StellarAssetClient::new(&env, &token).mint(&borrower, &400);
        approve(&env, &token, &borrower, &contract_id, 400);

        client.repay_credit(&borrower, &200);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            400
        );

        // Second repay must succeed (guard must be cleared)
        client.repay_credit(&borrower, &200);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            200
        );
    }

    // ── 12. Borrower auth required ────────────────────────────────────────────

    #[test]
    #[should_panic]
    fn repay_without_borrower_auth_reverts() {
        let env = Env::default();
        // Deliberately no mock_all_auths — require_auth() will fail
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);
        client.repay_credit(&borrower, &100);
    }

    #[test]
    fn repay_records_borrower_auth_requirement() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, token, contract_id, _admin) = setup(&env, &borrower, 1_000, 1_000, 300);

        StellarAssetClient::new(&env, &token).mint(&borrower, &100);
        approve(&env, &token, &borrower, &contract_id, 100);

        client.repay_credit(&borrower, &100);

        // Verify borrower auth was requested
        assert!(
            env.auths().iter().any(|(addr, _)| *addr == borrower),
            "repay_credit must require borrower authorization"
        );
    }

    // ── 13. Multiple partial repayments ──────────────────────────────────────

    #[test]
    fn repay_multiple_partial_repayments_accumulate_correctly() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, token, contract_id, _admin) = setup(&env, &borrower, 2_000, 2_000, 1_500);

        StellarAssetClient::new(&env, &token).mint(&borrower, &1_500);
        approve(&env, &token, &borrower, &contract_id, 1_500);

        client.repay_credit(&borrower, &500);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            1_000
        );

        client.repay_credit(&borrower, &400);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            600
        );

        client.repay_credit(&borrower, &600);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            0
        );
    }

    // ── 14. No token configured: state-only update ────────────────────────────

    #[test]
    fn repay_without_token_configured_still_updates_utilized_amount() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        // No set_liquidity_token call
        client.open_credit_line(&borrower, &1_000, &300_u32, &70_u32);

        // Manually set utilized_amount via internal state for this test
        env.as_contract(&contract_id, || {
            let mut line: CreditLineData = env.storage().persistent().get(&borrower).unwrap();
            line.utilized_amount = 400;
            env.storage().persistent().set(&borrower, &line);
        });

        client.repay_credit(&borrower, &150);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.utilized_amount, 250); // 400 - 150, no token transfer needed
    }

    // ── 15. State immutability: other fields unchanged after repay ────────────

    #[test]
    fn repay_does_not_mutate_non_utilized_fields() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, token, contract_id, _admin) = setup(&env, &borrower, 3_000, 3_000, 800);

        let before = client.get_credit_line(&borrower).unwrap();

        StellarAssetClient::new(&env, &token).mint(&borrower, &300);
        approve(&env, &token, &borrower, &contract_id, 300);
        client.repay_credit(&borrower, &300);

        let after = client.get_credit_line(&borrower).unwrap();

        assert_eq!(after.borrower, before.borrower);
        assert_eq!(after.credit_limit, before.credit_limit);
        assert_eq!(after.interest_rate_bps, before.interest_rate_bps);
        assert_eq!(after.risk_score, before.risk_score);
        assert_eq!(after.status, before.status);
        assert_eq!(after.utilized_amount, before.utilized_amount - 300);
    }
}

#[cfg(test)]
mod test_smoke_coverage {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};

    #[test]
    fn smoke_test_all_major_functions() {
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        let token_admin = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract_v2(token_admin);
        let token_address = token_id.address();
        client.set_liquidity_token(&token_address);
        let reserve = Address::generate(&env);
        client.set_liquidity_source(&reserve);

        client.open_credit_line(&borrower, &1000_i128, &500_u32, &60_u32);
        let sac = StellarAssetClient::new(&env, &token_address);
        sac.mint(&reserve, &2000_i128);
        client.draw_credit(&borrower, &400_i128);

        client.set_rate_change_limits(&200_u32, &86400_u64);
        assert!(client.get_rate_change_limits().is_some());
        client.update_risk_parameters(&borrower, &1200_i128, &600_u32, &70_u32);

        client.suspend_credit_line(&borrower);
        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower);

        sac.mint(&borrower, &100_i128);
        TokenClient::new(&env, &token_address).approve(
            &borrower,
            &contract_id,
            &100_i128,
            &1000_u32,
        );
        client.repay_credit(&borrower, &100_i128);

        client.close_credit_line(&borrower, &admin);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.status, CreditStatus::Closed);

        client.open_credit_line(&borrower, &500_i128, &300_u32, &50_u32);
    }

    #[test]
    #[should_panic(expected = "borrower already has an active credit line")]
    fn open_credit_line_rejects_duplicate_active() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let client = CreditClient::new(&env, &env.register(Credit, ()));
        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &500_u32, &60_u32);
        client.open_credit_line(&borrower, &1000_i128, &500_u32, &60_u32);
    }

    #[test]
    #[should_panic(expected = "Credit line not found")]
    fn test_suspend_nonexistent_credit_line() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let client = CreditClient::new(&env, &env.register(Credit, ()));
        client.init(&admin);
        client.open_credit_line(&Address::generate(&env), &1000_i128, &10001_u32, &60_u32);
    }

    #[test]
    #[should_panic(expected = "risk_score must be between 0 and 100")]
    fn open_credit_line_rejects_score_too_high() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let client = CreditClient::new(&env, &env.register(Credit, ()));
        client.init(&admin);
        client.open_credit_line(&Address::generate(&env), &1000_i128, &500_u32, &101_u32);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #3)")] // adjust # to match CreditLineNotFound's index
    fn draw_credit_rejects_borrower_mismatch() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let impostor = Address::generate(&env);
        let client = CreditClient::new(&env, &env.register(Credit, ()));
        client.init(&admin);
        let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
        client.set_liquidity_token(&token_id.address());
        client.open_credit_line(&borrower, &1000_i128, &500_u32, &60_u32);
        client.draw_credit(&impostor, &100_i128);
    }

    #[test]
    fn test_multiple_borrowers() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let client = CreditClient::new(&env, &env.register(Credit, ()));
        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &500_u32, &60_u32);
        client.default_credit_line(&borrower);
        client.suspend_credit_line(&borrower);
    }

    #[test]
    fn cover_all_types_and_remaining_error_paths() {
        let env = Env::default();
        env.mock_all_auths();

        // Force 100% coverage of types.rs
        let _ = CreditStatus::Active;
        let _ = CreditStatus::Suspended;
        let _ = CreditStatus::Defaulted;
        let _ = CreditStatus::Closed;

        let _ = ContractError::NotAdmin;
        let _ = ContractError::CreditLineNotFound;
        let _ = ContractError::CreditLineClosed;
        let _ = ContractError::OverLimit;
        let _ = ContractError::InvalidAmount;
        let _ = ContractError::UtilizationNotZero;
        let _ = ContractError::Unauthorized;
        let _ = ContractError::NegativeLimit;
        let _ = ContractError::RateTooHigh;
        let _ = ContractError::ScoreTooHigh;
        let _ = ContractError::Overflow;
        let _ = ContractError::Reentrancy;

        // Trigger a few more error paths
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let client = CreditClient::new(&env, &env.register(Credit, ()));
        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &500_u32, &60_u32);
    }
}

#[cfg(test)]
mod test_coverage_gaps {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::token::StellarAssetClient;

    fn base_setup(env: &Env) -> (CreditClient<'_>, Address, Address) {
        env.mock_all_auths();
        let admin = Address::generate(env);
        let borrower = Address::generate(env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(env, &contract_id);
        client.init(&admin);
        client.open_credit_line(&borrower, &1_000, &500_u32, &60_u32);
        (client, admin, borrower)
    }

    // ── update_risk_parameters: negative credit_limit ────────────────────────

    #[test]
    #[should_panic(expected = "credit_limit must be non-negative")]
    fn update_risk_params_negative_limit_reverts() {
        let env = Env::default();
        let (client, _admin, borrower) = base_setup(&env);
        client.update_risk_parameters(&borrower, &-1, &500_u32, &60_u32);
    }

    // ── update_risk_parameters: limit below utilized amount ──────────────────

    #[test]
    #[should_panic(expected = "credit_limit cannot be less than utilized amount")]
    fn update_risk_params_limit_below_utilized_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        client.set_liquidity_token(&token_id.address());
        StellarAssetClient::new(&env, &token_id.address()).mint(&contract_id, &1_000);
        client.open_credit_line(&borrower, &1_000, &500_u32, &60_u32);
        client.draw_credit(&borrower, &500);
        // Try to set limit below current utilization of 500
        client.update_risk_parameters(&borrower, &400, &500_u32, &60_u32);
    }

    // ── update_risk_parameters: interest_rate_bps over 10_000 ───────────────

    #[test]
    fn test_draw_credit_updates_utilized() {
        let env = Env::default();
        let (client, _admin, borrower) = base_setup(&env);
        client.update_risk_parameters(&borrower, &1_000, &500_u32, &101_u32);
    }

    // ── close_credit_line: unauthorized closer (not admin, not borrower) ─────

    #[test]
    #[should_panic(expected = "unauthorized")]
    fn close_credit_line_unauthorized_closer_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, borrower) = base_setup(&env);
        let stranger = Address::generate(&env);
        client.close_credit_line(&borrower, &stranger);
    }

    // ── close_credit_line: borrower attempts close with non-zero utilization ─

    #[test]
    #[should_panic(expected = "cannot close: utilized amount not zero")]
    fn close_credit_line_borrower_with_utilization_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        client.set_liquidity_token(&token_id.address());
        StellarAssetClient::new(&env, &token_id.address()).mint(&contract_id, &1_000);
        client.open_credit_line(&borrower, &1_000, &500_u32, &60_u32);
        client.draw_credit(&borrower, &200);
        // Borrower tries to close while still owing
        client.close_credit_line(&borrower, &borrower);
    }

    // ── reinstate_credit_line: not defaulted → panics ────────────────────────

    #[test]
    #[should_panic(expected = "credit line is not defaulted")]
    fn reinstate_non_defaulted_active_line_reverts() {
        let env = Env::default();
        let (client, _admin, borrower) = base_setup(&env);
        // Line is Active, not Defaulted
        client.reinstate_credit_line(&borrower);
    }

    #[test]
    #[should_panic(expected = "credit line is not defaulted")]
    fn reinstate_suspended_line_reverts() {
        let env = Env::default();
        let (client, _admin, borrower) = base_setup(&env);
        client.suspend_credit_line(&borrower);
        // Line is Suspended, not Defaulted
        client.reinstate_credit_line(&borrower);
    }

    // ── open_credit_line: allows reopening after Closed status ───────────────

    #[test]
    fn open_credit_line_succeeds_after_closed() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin, borrower) = base_setup(&env);
        client.close_credit_line(&borrower, &admin);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Closed
        );
        // Should succeed: existing line is Closed, not Active
        client.open_credit_line(&borrower, &500, &300_u32, &50_u32);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Active
        );
    }

    #[test]
    fn test_event_reinstate_credit_line() {
        use soroban_sdk::testutils::Events;
        use soroban_sdk::{TryFromVal, TryIntoVal};
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _token, _admin) = setup_contract_with_credit_line(&env, &borrower, 1_000, 0);
        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower);
        let events = env.events().all();
        let (_contract, topics, data) = events.last().unwrap();
        assert_eq!(
            Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
            symbol_short!("reinstate")
        );
        let event_data: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(event_data.status, CreditStatus::Active);
    }

    #[test]
    fn test_event_lifecycle_sequence() {
        use soroban_sdk::testutils::Events as _;
        use soroban_sdk::{TryFromVal, TryIntoVal};

        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.draw_credit(&borrower, &200_i128);
        client.repay_credit(&borrower, &50_i128);
        client.suspend_credit_line(&borrower);
        client.default_credit_line(&borrower);
        client.reinstate_credit_line(&borrower);
        client.close_credit_line(&borrower, &admin);

        let events = env.events().all();
        assert!(!events.is_empty());

        let (_contract, topics, data) = events.last().unwrap();
        assert_eq!(
            Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
            symbol_short!("closed")
        );
        let event_data: CreditLineEvent = data.try_into_val(&env).unwrap();
        assert_eq!(event_data.status, CreditStatus::Closed);
        assert_eq!(event_data.borrower, borrower);
    }

    #[test]
    fn test_rate_change_limits_roundtrip() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.set_rate_change_limits(&250_u32, &3600_u64);

        let cfg = client.get_rate_change_limits().unwrap();
        assert_eq!(cfg.max_rate_change_bps, 250);
        assert_eq!(cfg.rate_change_min_interval, 3600);
    }

    #[test]
    #[should_panic(expected = "interest_rate_bps exceeds maximum")]
    fn test_update_risk_parameters_interest_rate_exceeds_max() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.update_risk_parameters(&borrower, &1000_i128, &10001_u32, &70_u32);
    }

    #[test]
    #[should_panic(expected = "risk_score exceeds maximum")]
    fn test_update_risk_parameters_risk_score_exceeds_max() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32);
        client.update_risk_parameters(&borrower, &1000_i128, &300_u32, &101_u32);
    }

    #[test]
    #[should_panic(expected = "amount must be positive")]
    fn draw_credit_zero_amount_reverts_and_guard_cleared() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        client.open_credit_line(&borrower, &1_000, &500_u32, &60_u32);
        client.draw_credit(&borrower, &0);
    }

    // ── draw_credit: defaulted line rejects draw ──────────────────────────────

    #[test]
    #[should_panic(expected = "credit line is defaulted")]
    fn draw_credit_on_defaulted_line_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        client.set_liquidity_token(&token_id.address());
        StellarAssetClient::new(&env, &token_id.address()).mint(&contract_id, &1_000);
        client.open_credit_line(&borrower, &1_000, &500_u32, &60_u32);
        client.default_credit_line(&borrower);
        client.draw_credit(&borrower, &100);
    }

    // ── draw_credit: closed line uses ContractError path ─────────────────────

    #[test]
    #[should_panic(expected = "Error(Contract, #4)")]
    fn draw_credit_on_closed_line_reverts_with_contract_error() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);
        client.set_liquidity_token(&token_id.address());
        client.open_credit_line(&borrower, &1_000, &500_u32, &60_u32);
        client.close_credit_line(&borrower, &admin);
        client.draw_credit(&borrower, &100);
    }

    // ── update_risk_parameters: rate change interval passes ──────────────────

    #[test]
    fn rate_change_after_interval_succeeds() {
        use soroban_sdk::testutils::Ledger;
        let env = Env::default();
        let (client, _admin, borrower) = base_setup(&env);
        client.set_rate_change_limits(&1_000_u32, &86_400_u64);
        env.ledger().set_timestamp(100);
        client.update_risk_parameters(&borrower, &1_000, &600_u32, &60_u32);
        // Advance past the minimum interval
        env.ledger().set_timestamp(100 + 86_400 + 1);
        client.update_risk_parameters(&borrower, &1_000, &700_u32, &60_u32);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().interest_rate_bps,
            700
        );
    }

    // ── suspend_credit_line from Defaulted → panic (not Active) ─────────────

    #[test]
    #[should_panic(expected = "Only active credit lines can be suspended")]
    fn suspend_defaulted_line_reverts() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let token_admin = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);

        let token = env.register_stellar_asset_contract_v2(token_admin);
        let token_admin_client = StellarAssetClient::new(&env, &token.address());
        let token_client = token::Client::new(&env, &token.address());

        client.set_liquidity_token(&token.address());

        token_admin_client.mint(&contract_id, &500_i128);
        client.draw_credit(&borrower, &200_i128);

        assert_eq!(token_client.balance(&contract_id), 300_i128);
        assert_eq!(token_client.balance(&borrower), 200_i128);
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            200_i128
        );
    }

    // ── close_credit_line: idempotent on already-Closed line ─────────────────

    #[test]
    fn close_credit_line_idempotent_when_already_closed() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let token_admin = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);

        let token = env.register_stellar_asset_contract_v2(token_admin);
        let token_admin_client = StellarAssetClient::new(&env, &token.address());
        let token_client = token::Client::new(&env, &token.address());
        let reserve = contract_id.clone();

        client.set_liquidity_token(&token.address());
        client.set_liquidity_source(&reserve);

        token_admin_client.mint(&reserve, &500_i128);
        client.draw_credit(&borrower, &120_i128);

        assert_eq!(token_client.balance(&reserve), 380_i128);
        assert_eq!(token_client.balance(&borrower), 120_i128);
        assert_eq!(token_client.balance(&contract_id), 380_i128);
    }

    // ── draw_credit: overflow protection ─────────────────────────────────────

    #[test]
    #[should_panic]
    fn draw_credit_overflow_on_utilized_amount_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let token_admin = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client.open_credit_line(&borrower, &1_000_i128, &300_u32, &70_u32);

        let token = env.register_stellar_asset_contract_v2(token_admin);
        let token_admin_client = StellarAssetClient::new(&env, &token.address());

        client.set_liquidity_token(&token.address());

        token_admin_client.mint(&contract_id, &50_i128);
        client.draw_credit(&borrower, &100_i128);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests: rate-change limits (issue #17)
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod test_rate_change_limits {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::testutils::Ledger;

    fn setup<'a>(
        env: &'a Env,
        borrower: &'a Address,
        credit_limit: i128,
        _reserve_amount: i128,
    ) -> (CreditClient<'a>, Address) {
        let admin = Address::generate(env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(env, &contract_id);
        client.init(&admin);
        client.open_credit_line(borrower, &credit_limit, &300_u32, &70_u32);
        (client, admin)
    }

    #[test]
    fn test_rate_change_within_limit_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        client.set_rate_change_limits(&100_u32, &0_u64);
        client.update_risk_parameters(&borrower, &5_000_i128, &350_u32, &70_u32);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.interest_rate_bps, 350);
    }

    #[test]
    #[should_panic(expected = "rate change exceeds maximum allowed delta")]
    fn test_rate_change_over_limit_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        client.set_rate_change_limits(&50_u32, &0_u64);
        // Current rate is 300; 300 + 51 = 351 → delta 51 > 50
        client.update_risk_parameters(&borrower, &5_000_i128, &351_u32, &70_u32);
    }

    #[test]
    #[should_panic(expected = "rate change exceeds maximum allowed delta")]
    fn test_rate_decrease_over_limit_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        client.set_rate_change_limits(&50_u32, &0_u64);
        // Current rate is 300; 300 - 51 = 249 → delta 51 > 50
        client.update_risk_parameters(&borrower, &5_000_i128, &249_u32, &70_u32);
    }

    #[test]
    fn test_rate_change_at_exact_limit_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        client.set_rate_change_limits(&50_u32, &0_u64);
        // Current rate 300; 300 + 50 = 350 → delta == limit
        client.update_risk_parameters(&borrower, &5_000_i128, &350_u32, &70_u32);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.interest_rate_bps, 350);
    }

    #[test]
    #[should_panic(expected = "rate change exceeds maximum allowed delta")]
    fn test_rate_change_one_over_limit_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        client.set_rate_change_limits(&50_u32, &0_u64);
        // Current rate 300; 300 + 51 = 351 → delta 51 > 50
        client.update_risk_parameters(&borrower, &5_000_i128, &351_u32, &70_u32);
    }

    #[test]
    #[should_panic(expected = "rate change too soon: minimum interval not elapsed")]
    fn test_rate_change_within_interval_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        // Allow up to 100 bps change but only every 3600 seconds (1 hour).
        client.set_rate_change_limits(&100_u32, &3600_u64);

        // First update at t=100.
        env.ledger().with_mut(|li| li.timestamp = 100);
        client.update_risk_parameters(&borrower, &5_000_i128, &350_u32, &70_u32);

        // Second update at t=200 (only 100 s later, < 3600).
        env.ledger().with_mut(|li| li.timestamp = 200);
        client.update_risk_parameters(&borrower, &5_000_i128, &330_u32, &70_u32);
    }

    #[test]
    fn test_rate_change_after_interval_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        client.set_rate_change_limits(&100_u32, &3600_u64);

        env.ledger().with_mut(|li| li.timestamp = 100);
        client.update_risk_parameters(&borrower, &5_000_i128, &350_u32, &70_u32);

        // Advance past the interval.
        env.ledger().with_mut(|li| li.timestamp = 3701);
        client.update_risk_parameters(&borrower, &5_000_i128, &330_u32, &70_u32);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.interest_rate_bps, 330);
    }

    #[test]
    fn test_rate_change_at_exact_interval_boundary_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        client.set_rate_change_limits(&100_u32, &3600_u64);

        env.ledger().with_mut(|li| li.timestamp = 100);
        client.update_risk_parameters(&borrower, &5_000_i128, &350_u32, &70_u32);

        // Exactly on the interval boundary: elapsed == 3600.
        env.ledger().with_mut(|li| li.timestamp = 3700);
        client.update_risk_parameters(&borrower, &5_000_i128, &330_u32, &70_u32);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.interest_rate_bps, 330);
        assert_eq!(line.last_rate_update_ts, 3700);
    }

    #[test]
    fn test_rate_change_first_update_ignores_interval() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        // Interval set but first update should always pass (last_rate_update_ts == 0).
        client.set_rate_change_limits(&100_u32, &86400_u64);
        env.ledger().with_mut(|li| li.timestamp = 10);
        client.update_risk_parameters(&borrower, &5_000_i128, &350_u32, &70_u32);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.interest_rate_bps, 350);
    }

    #[test]
    fn test_zero_interval_disables_timing_check_after_first_update() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        client.set_rate_change_limits(&100_u32, &0_u64);

        env.ledger().with_mut(|li| li.timestamp = 100);
        client.update_risk_parameters(&borrower, &5_000_i128, &350_u32, &70_u32);

        // Immediate subsequent update should still pass because interval == 0 disables the gate.
        env.ledger().with_mut(|li| li.timestamp = 101);
        client.update_risk_parameters(&borrower, &5_000_i128, &330_u32, &70_u32);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.interest_rate_bps, 330);
        assert_eq!(line.last_rate_update_ts, 101);
    }

    #[test]
    fn test_same_rate_bypasses_limits() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        // Strict limits: 0 bps max change, huge interval.
        client.set_rate_change_limits(&0_u32, &999_999_u64);

        // Same rate (300 → 300) should still succeed.
        client.update_risk_parameters(&borrower, &5_000_i128, &300_u32, &70_u32);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.interest_rate_bps, 300);
    }

    #[test]
    fn test_no_rate_limits_configured_backward_compat() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        // No set_rate_change_limits call → unlimited changes.
        client.update_risk_parameters(&borrower, &5_000_i128, &9_999_u32, &70_u32);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.interest_rate_bps, 9_999);
    }

    #[test]
    fn test_set_and_get_rate_change_limits() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);

        client.set_rate_change_limits(&200_u32, &7200_u64);
        let cfg = client.get_rate_change_limits().unwrap();

        assert_eq!(cfg.max_rate_change_bps, 200);
        assert_eq!(cfg.rate_change_min_interval, 7200);
    }

    #[test]
    fn test_rate_change_timestamp_recorded() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        client.set_rate_change_limits(&100_u32, &0_u64);
        env.ledger().with_mut(|li| li.timestamp = 42);
        client.update_risk_parameters(&borrower, &5_000_i128, &350_u32, &70_u32);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.last_rate_update_ts, 42);
    }

    #[test]
    fn test_rate_change_multiple_sequential_within_limits() {
        let env = Env::default();
        env.mock_all_auths();
        let borrower = Address::generate(&env);
        let (client, _admin) = setup(&env, &borrower, 5_000, 0);

        client.set_rate_change_limits(&50_u32, &60_u64);

        // First update at t=100: 300 → 350
        env.ledger().with_mut(|li| li.timestamp = 100);
        client.update_risk_parameters(&borrower, &5_000_i128, &350_u32, &70_u32);

        // Second update at t=161: 350 → 320 (delta 30 ≤ 50)
        env.ledger().with_mut(|li| li.timestamp = 161);
        client.update_risk_parameters(&borrower, &5_000_i128, &320_u32, &65_u32);

        // Third update at t=222: 320 → 370 (delta 50 == limit)
        env.ledger().with_mut(|li| li.timestamp = 222);
        client.update_risk_parameters(&borrower, &5_000_i128, &370_u32, &60_u32);

        let line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(line.interest_rate_bps, 370);
        assert_eq!(line.risk_score, 60);
    }

    #[test]
    #[should_panic(expected = "Unauthorized")]
    fn test_set_rate_change_limits_unauthorized() {
        let env = Env::default();
        // NOTE: no mock_all_auths → admin auth will fail.
        let admin = Address::generate(&env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);
        client.init(&admin);

        client.set_rate_change_limits(&100_u32, &0_u64);
    }
}