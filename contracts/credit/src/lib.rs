#![no_std]

//! Creditra credit contract: credit lines, draw/repay, risk parameters.
//!
//! # Error handling
//! All entry-points return `Result<_, CreditError>` so that callers receive a
//! stable, numeric error code rather than an opaque panic string.  See
//! [`errors::CreditError`] for the full table of codes.
//!
//! # Reentrancy
//! Soroban token transfers (e.g. Stellar Asset Contract) do not invoke
//! callbacks back into the caller.  This contract uses a reentrancy guard on
//! [`Credit::draw_credit`] and [`Credit::repay_credit`] as a defense-in-depth
//! measure; if a token or future integration ever called back, the guard would
//! return [`CreditError::ReentrancyGuard`].

mod errors;
mod events;
mod types;

use soroban_sdk::{contract, contractimpl, symbol_short, Address, Env, Symbol};

use errors::CreditError;
use events::{
    publish_credit_line_event, publish_repayment_event, publish_risk_parameters_updated,
    CreditLineEvent, RepaymentEvent, RiskParametersUpdatedEvent,
};
use types::{CreditLineData, CreditStatus};

// ── Constants ────────────────────────────────────────────────────────────────

/// Maximum interest rate in basis points (100 %).
const MAX_INTEREST_RATE_BPS: u32 = 100_00;
/// Maximum risk score (0–100 scale).
const MAX_RISK_SCORE: u32 = 100;

// ── Storage key helpers ───────────────────────────────────────────────────────

/// Instance storage key for the reentrancy guard flag.
fn reentrancy_key(env: &Env) -> Symbol {
    Symbol::new(env, "reentrancy")
}

/// Instance storage key for the contract admin address.
fn admin_key(env: &Env) -> Symbol {
    Symbol::new(env, "admin")
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Return the stored admin address, or panic if the contract has not been
/// initialised yet.  (Uninitialised state is a deployment error, not a
/// user-facing error, so a plain `expect` is acceptable here.)
fn require_admin(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&admin_key(env))
        .expect("admin not set")
}

/// Require that the caller is the contract admin and has authorised this
/// invocation.  Returns [`CreditError::Unauthorized`] otherwise.
fn require_admin_auth(env: &Env) -> Result<Address, CreditError> {
    let admin = require_admin(env);
    admin.require_auth();
    Ok(admin)
}

/// Assert the reentrancy guard is not already set, then set it.
///
/// # Errors
/// Returns [`CreditError::ReentrancyGuard`] if the guard is already active.
fn set_reentrancy_guard(env: &Env) -> Result<(), CreditError> {
    let key = reentrancy_key(env);
    let current: bool = env.storage().instance().get(&key).unwrap_or(false);
    if current {
        return Err(CreditError::ReentrancyGuard);
    }
    env.storage().instance().set(&key, &true);
    Ok(())
}

/// Clear the reentrancy guard (must be called on every exit path after
/// [`set_reentrancy_guard`] succeeds).
fn clear_reentrancy_guard(env: &Env) {
    env.storage().instance().set(&reentrancy_key(env), &false);
}

// ── Contract ──────────────────────────────────────────────────────────────────

#[contract]
pub struct Credit;

#[contractimpl]
impl Credit {
    // ── Initialisation ────────────────────────────────────────────────────────

    /// Initialise the contract by recording the admin address.
    ///
    /// This must be called exactly once immediately after deployment.
    pub fn init(env: Env, admin: Address) {
        env.storage().instance().set(&admin_key(&env), &admin);
    }

    // ── Credit-line management ────────────────────────────────────────────────

    /// Open a new credit line for `borrower` (called by the backend / risk engine).
    ///
    /// # Arguments
    /// * `borrower`          – Address that will own the credit line.
    /// * `credit_limit`      – Maximum drawable amount (must be ≥ 0).
    /// * `interest_rate_bps` – Annual interest rate in basis points (0 ..= 10 000).
    /// * `risk_score`        – Risk score (0 ..= 100).
    ///
    /// # Errors
    /// * [`CreditError::InvalidCreditLimit`]    – `credit_limit` is negative.
    /// * [`CreditError::InterestRateExceedsMax`] – `interest_rate_bps` > 10 000.
    /// * [`CreditError::RiskScoreExceedsMax`]   – `risk_score` > 100.
    ///
    /// Emits a `CreditLineOpened` event.
    pub fn open_credit_line(
        env: Env,
        borrower: Address,
        credit_limit: i128,
        interest_rate_bps: u32,
        risk_score: u32,
    ) -> Result<(), CreditError> {
        if credit_limit < 0 {
            return Err(CreditError::InvalidCreditLimit);
        }
        if interest_rate_bps > MAX_INTEREST_RATE_BPS {
            return Err(CreditError::InterestRateExceedsMax);
        }
        if risk_score > MAX_RISK_SCORE {
            return Err(CreditError::RiskScoreExceedsMax);
        }

        let credit_line = CreditLineData {
            borrower: borrower.clone(),
            credit_limit,
            utilized_amount: 0,
            interest_rate_bps,
            risk_score,
            status: CreditStatus::Active,
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
        Ok(())
    }

    /// Draw `amount` from the borrower's credit line.
    ///
    /// # Arguments
    /// * `borrower` – Must have authorised this invocation.
    /// * `amount`   – Strictly positive amount to draw.
    ///
    /// # Errors
    /// * [`CreditError::ReentrancyGuard`]  – Reentrant call detected.
    /// * [`CreditError::CreditLineNotFound`] – No credit line for `borrower`.
    /// * [`CreditError::CreditLineClosed`] – Credit line is closed.
    /// * [`CreditError::InvalidAmount`]    – `amount` ≤ 0.
    /// * [`CreditError::OverLimit`]        – Draw would exceed `credit_limit`.
    /// * [`CreditError::Overflow`]         – Arithmetic overflow.
    pub fn draw_credit(env: Env, borrower: Address, amount: i128) -> Result<(), CreditError> {
        set_reentrancy_guard(&env)?;
        borrower.require_auth();

        let result = (|| -> Result<(), CreditError> {
            let mut credit_line: CreditLineData = env
                .storage()
                .persistent()
                .get(&borrower)
                .ok_or(CreditError::CreditLineNotFound)?;

            if credit_line.status == CreditStatus::Closed {
                return Err(CreditError::CreditLineClosed);
            }
            if amount <= 0 {
                return Err(CreditError::InvalidAmount);
            }
            let new_utilized = credit_line
                .utilized_amount
                .checked_add(amount)
                .ok_or(CreditError::Overflow)?;
            if new_utilized > credit_line.credit_limit {
                return Err(CreditError::OverLimit);
            }
            credit_line.utilized_amount = new_utilized;
            env.storage().persistent().set(&borrower, &credit_line);
            // TODO: transfer token to borrower
            Ok(())
        })();

        clear_reentrancy_guard(&env);
        result
    }

    /// Repay `amount` against the borrower's credit line.
    ///
    /// Reduces `utilized_amount` by `amount`, saturating at zero.
    /// Emits a [`RepaymentEvent`].
    ///
    /// # Arguments
    /// * `borrower` – Must have authorised this invocation.
    /// * `amount`   – Strictly positive amount to repay.
    ///
    /// # Errors
    /// * [`CreditError::ReentrancyGuard`]    – Reentrant call detected.
    /// * [`CreditError::CreditLineNotFound`] – No credit line for `borrower`.
    /// * [`CreditError::CreditLineClosed`]   – Credit line is closed.
    /// * [`CreditError::InvalidAmount`]      – `amount` ≤ 0.
    pub fn repay_credit(env: Env, borrower: Address, amount: i128) -> Result<(), CreditError> {
        set_reentrancy_guard(&env)?;
        borrower.require_auth();

        let result = (|| -> Result<(), CreditError> {
            let mut credit_line: CreditLineData = env
                .storage()
                .persistent()
                .get(&borrower)
                .ok_or(CreditError::CreditLineNotFound)?;

            if credit_line.status == CreditStatus::Closed {
                return Err(CreditError::CreditLineClosed);
            }
            if amount <= 0 {
                return Err(CreditError::InvalidAmount);
            }
            let new_utilized = credit_line.utilized_amount.saturating_sub(amount).max(0);
            credit_line.utilized_amount = new_utilized;
            env.storage().persistent().set(&borrower, &credit_line);

            let timestamp = env.ledger().timestamp();
            publish_repayment_event(
                &env,
                RepaymentEvent {
                    borrower: borrower.clone(),
                    amount,
                    new_utilized_amount: new_utilized,
                    timestamp,
                },
            );
            // TODO: accept token from borrower
            Ok(())
        })();

        clear_reentrancy_guard(&env);
        result
    }

    /// Update risk parameters for an existing credit line (admin only).
    ///
    /// # Arguments
    /// * `borrower`          – Borrower whose credit line to update.
    /// * `credit_limit`      – New credit limit (must be ≥ `utilized_amount` and ≥ 0).
    /// * `interest_rate_bps` – New interest rate in basis points (0 ..= 10 000).
    /// * `risk_score`        – New risk score (0 ..= 100).
    ///
    /// # Errors
    /// * [`CreditError::Unauthorized`]          – Caller is not the contract admin.
    /// * [`CreditError::CreditLineNotFound`]    – No credit line for `borrower`.
    /// * [`CreditError::InvalidCreditLimit`]    – `credit_limit` < 0 or < `utilized_amount`.
    /// * [`CreditError::InterestRateExceedsMax`] – `interest_rate_bps` > 10 000.
    /// * [`CreditError::RiskScoreExceedsMax`]   – `risk_score` > 100.
    ///
    /// Emits a `RiskParametersUpdated` event.
    pub fn update_risk_parameters(
        env: Env,
        borrower: Address,
        credit_limit: i128,
        interest_rate_bps: u32,
        risk_score: u32,
    ) -> Result<(), CreditError> {
        require_admin_auth(&env)?;

        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .ok_or(CreditError::CreditLineNotFound)?;

        if credit_limit < 0 {
            return Err(CreditError::InvalidCreditLimit);
        }
        if credit_limit < credit_line.utilized_amount {
            return Err(CreditError::InvalidCreditLimit);
        }
        if interest_rate_bps > MAX_INTEREST_RATE_BPS {
            return Err(CreditError::InterestRateExceedsMax);
        }
        if risk_score > MAX_RISK_SCORE {
            return Err(CreditError::RiskScoreExceedsMax);
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
        Ok(())
    }

    /// Suspend a credit line (admin only).
    ///
    /// # Errors
    /// * [`CreditError::Unauthorized`]       – Caller is not the contract admin.
    /// * [`CreditError::CreditLineNotFound`] – No credit line for `borrower`.
    ///
    /// Emits a `CreditLineSuspended` event.
    pub fn suspend_credit_line(env: Env, borrower: Address) -> Result<(), CreditError> {
        require_admin_auth(&env)?;

        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .ok_or(CreditError::CreditLineNotFound)?;

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
        Ok(())
    }

    /// Close a credit line.
    ///
    /// Callable by the contract admin (force-close regardless of utilization)
    /// or by the borrower when `utilized_amount` is zero.  If the line is
    /// already closed the call is a no-op (idempotent).
    ///
    /// # Arguments
    /// * `borrower` – Owner of the credit line.
    /// * `closer`   – Address authorising the close; must be admin or borrower.
    ///
    /// # Errors
    /// * [`CreditError::CreditLineNotFound`]    – No credit line for `borrower`.
    /// * [`CreditError::Unauthorized`]          – `closer` is neither admin nor borrower.
    /// * [`CreditError::UtilizedAmountNotZero`] – Borrower tried to close while
    ///   `utilized_amount != 0`.
    ///
    /// Emits a `CreditLineClosed` event (unless already closed).
    pub fn close_credit_line(
        env: Env,
        borrower: Address,
        closer: Address,
    ) -> Result<(), CreditError> {
        closer.require_auth();

        let admin: Address = require_admin(&env);

        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .ok_or(CreditError::CreditLineNotFound)?;

        // Idempotent: already closed → no-op.
        if credit_line.status == CreditStatus::Closed {
            return Ok(());
        }

        if closer == admin {
            // Admin can force-close regardless of utilization.
        } else if closer == borrower {
            // Borrower can only close when fully repaid.
            if credit_line.utilized_amount != 0 {
                return Err(CreditError::UtilizedAmountNotZero);
            }
        } else {
            return Err(CreditError::Unauthorized);
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
        Ok(())
    }

    /// Mark a credit line as defaulted (admin only).
    ///
    /// # Errors
    /// * [`CreditError::Unauthorized`]       – Caller is not the contract admin.
    /// * [`CreditError::CreditLineNotFound`] – No credit line for `borrower`.
    ///
    /// Emits a `CreditLineDefaulted` event.
    pub fn default_credit_line(env: Env, borrower: Address) -> Result<(), CreditError> {
        require_admin_auth(&env)?;

        let mut credit_line: CreditLineData = env
            .storage()
            .persistent()
            .get(&borrower)
            .ok_or(CreditError::CreditLineNotFound)?;

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
        Ok(())
    }

    /// Return the credit line data for `borrower`, or `None` if it does not exist.
    pub fn get_credit_line(env: Env, borrower: Address) -> Option<CreditLineData> {
        env.storage().persistent().get(&borrower)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::testutils::Events;

    // ── helpers ───────────────────────────────────────────────────────────────

    /// Unwrap a contract `Result`, panicking with the error variant on failure.
    /// This keeps test bodies concise while still surfacing the exact error.
    fn unwrap_ok<T>(r: Result<T, CreditError>) -> T {
        r.unwrap_or_else(|e| panic!("unexpected error: {:?}", e))
    }

    // ── basic lifecycle ───────────────────────────────────────────────────────

    #[test]
    fn test_init_and_open_credit_line() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32));

        let credit_line = client.get_credit_line(&borrower);
        assert!(credit_line.is_some());
        let credit_line = credit_line.unwrap();
        assert_eq!(credit_line.borrower, borrower);
        assert_eq!(credit_line.credit_limit, 1000);
        assert_eq!(credit_line.utilized_amount, 0);
        assert_eq!(credit_line.interest_rate_bps, 300);
        assert_eq!(credit_line.risk_score, 70);
        assert_eq!(credit_line.status, CreditStatus::Active);
    }

    #[test]
    fn test_suspend_credit_line() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32));
        unwrap_ok(client.suspend_credit_line(&borrower));

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.status, CreditStatus::Suspended);
    }

    #[test]
    fn test_close_credit_line() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32));
        unwrap_ok(client.close_credit_line(&borrower, &admin));

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.status, CreditStatus::Closed);
    }

    #[test]
    fn test_default_credit_line() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32));
        unwrap_ok(client.default_credit_line(&borrower));

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.status, CreditStatus::Defaulted);
    }

    #[test]
    fn test_full_lifecycle() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);

        unwrap_ok(client.open_credit_line(&borrower, &5000_i128, &500_u32, &80_u32));
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Active
        );

        unwrap_ok(client.suspend_credit_line(&borrower));
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Suspended
        );

        unwrap_ok(client.close_credit_line(&borrower, &admin));
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Closed
        );
    }

    #[test]
    fn test_event_data_integrity() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower, &2000_i128, &400_u32, &75_u32));

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.borrower, borrower);
        assert_eq!(credit_line.status, CreditStatus::Active);
        assert_eq!(credit_line.credit_limit, 2000);
        assert_eq!(credit_line.interest_rate_bps, 400);
        assert_eq!(credit_line.risk_score, 75);
    }

    // ── CreditLineNotFound errors ─────────────────────────────────────────────

    /// `suspend_credit_line` returns `CreditLineNotFound` for an unknown borrower.
    #[test]
    fn test_suspend_nonexistent_credit_line() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        let err = client
            .try_suspend_credit_line(&borrower)
            .expect_err("should fail");
        assert_eq!(
            err.unwrap(),
            CreditError::CreditLineNotFound,
            "expected CreditLineNotFound (code 2)"
        );
    }

    /// `close_credit_line` returns `CreditLineNotFound` for an unknown borrower.
    #[test]
    fn test_close_nonexistent_credit_line() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        let err = client
            .try_close_credit_line(&borrower, &admin)
            .expect_err("should fail");
        assert_eq!(
            err.unwrap(),
            CreditError::CreditLineNotFound,
            "expected CreditLineNotFound (code 2)"
        );
    }

    /// `default_credit_line` returns `CreditLineNotFound` for an unknown borrower.
    #[test]
    fn test_default_nonexistent_credit_line() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        let err = client
            .try_default_credit_line(&borrower)
            .expect_err("should fail");
        assert_eq!(
            err.unwrap(),
            CreditError::CreditLineNotFound,
            "expected CreditLineNotFound (code 2)"
        );
    }

    // ── Multiple borrowers ────────────────────────────────────────────────────

    #[test]
    fn test_multiple_borrowers() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower1 = Address::generate(&env);
        let borrower2 = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower1, &1000_i128, &300_u32, &70_u32));
        unwrap_ok(client.open_credit_line(&borrower2, &2000_i128, &400_u32, &80_u32));

        let credit_line1 = client.get_credit_line(&borrower1).unwrap();
        let credit_line2 = client.get_credit_line(&borrower2).unwrap();

        assert_eq!(credit_line1.credit_limit, 1000);
        assert_eq!(credit_line2.credit_limit, 2000);
        assert_eq!(credit_line1.status, CreditStatus::Active);
        assert_eq!(credit_line2.status, CreditStatus::Active);
    }

    // ── Lifecycle transitions ─────────────────────────────────────────────────

    #[test]
    fn test_lifecycle_transitions() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);

        unwrap_ok(client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32));
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Active
        );

        unwrap_ok(client.default_credit_line(&borrower));
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Defaulted
        );
    }

    // ── close_credit_line ─────────────────────────────────────────────────────

    /// Borrower can close their own line when utilization is zero.
    #[test]
    fn test_close_credit_line_borrower_when_utilized_zero() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32));
        unwrap_ok(client.close_credit_line(&borrower, &borrower));

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.status, CreditStatus::Closed);
        assert_eq!(credit_line.utilized_amount, 0);
    }

    /// Borrower receives `UtilizedAmountNotZero` when trying to close with outstanding balance.
    #[test]
    fn test_close_credit_line_borrower_rejected_when_utilized_nonzero() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32));
        unwrap_ok(client.draw_credit(&borrower, &300_i128));

        let err = client
            .try_close_credit_line(&borrower, &borrower)
            .expect_err("should fail");
        assert_eq!(
            err.unwrap(),
            CreditError::UtilizedAmountNotZero,
            "expected UtilizedAmountNotZero (code 6)"
        );
    }

    /// Admin can force-close a line that still has utilization.
    #[test]
    fn test_close_credit_line_admin_force_close_with_utilization() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32));
        unwrap_ok(client.draw_credit(&borrower, &300_i128));
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            300
        );

        unwrap_ok(client.close_credit_line(&borrower, &admin));

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.status, CreditStatus::Closed);
        assert_eq!(credit_line.utilized_amount, 300);
    }

    /// Closing an already-closed line is a no-op (idempotent).
    #[test]
    fn test_close_credit_line_idempotent_when_already_closed() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32));
        unwrap_ok(client.close_credit_line(&borrower, &admin));
        unwrap_ok(client.close_credit_line(&borrower, &admin));

        assert_eq!(
            client.get_credit_line(&borrower).unwrap().status,
            CreditStatus::Closed
        );
    }

    /// A third-party address receives `Unauthorized` when trying to close.
    #[test]
    fn test_close_credit_line_unauthorized_closer() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);
        let other = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32));

        let err = client
            .try_close_credit_line(&borrower, &other)
            .expect_err("should fail");
        assert_eq!(
            err.unwrap(),
            CreditError::Unauthorized,
            "expected Unauthorized (code 1)"
        );
    }

    // ── draw_credit ───────────────────────────────────────────────────────────

    /// `draw_credit` returns `CreditLineClosed` on a closed line.
    #[test]
    fn test_draw_credit_rejected_when_closed() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32));
        unwrap_ok(client.close_credit_line(&borrower, &admin));

        let err = client
            .try_draw_credit(&borrower, &100_i128)
            .expect_err("should fail");
        assert_eq!(
            err.unwrap(),
            CreditError::CreditLineClosed,
            "expected CreditLineClosed (code 5)"
        );
    }

    /// `draw_credit` updates `utilized_amount` correctly across multiple draws.
    #[test]
    fn test_draw_credit_updates_utilized() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32));

        unwrap_ok(client.draw_credit(&borrower, &200_i128));
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            200
        );

        unwrap_ok(client.draw_credit(&borrower, &300_i128));
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            500
        );
    }

    /// `draw_credit` returns `OverLimit` when the draw would exceed the credit limit.
    #[test]
    fn test_draw_credit_over_limit() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower, &500_i128, &300_u32, &70_u32));

        let err = client
            .try_draw_credit(&borrower, &600_i128)
            .expect_err("should fail");
        assert_eq!(
            err.unwrap(),
            CreditError::OverLimit,
            "expected OverLimit (code 4)"
        );
    }

    /// `draw_credit` returns `InvalidAmount` for a zero amount.
    #[test]
    fn test_draw_credit_invalid_amount_zero() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32));

        let err = client
            .try_draw_credit(&borrower, &0_i128)
            .expect_err("should fail");
        assert_eq!(
            err.unwrap(),
            CreditError::InvalidAmount,
            "expected InvalidAmount (code 3)"
        );
    }

    /// `draw_credit` returns `CreditLineNotFound` for an unknown borrower.
    #[test]
    fn test_draw_credit_nonexistent_line() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);

        let err = client
            .try_draw_credit(&borrower, &100_i128)
            .expect_err("should fail");
        assert_eq!(
            err.unwrap(),
            CreditError::CreditLineNotFound,
            "expected CreditLineNotFound (code 2)"
        );
    }

    // ── update_risk_parameters ────────────────────────────────────────────────

    #[test]
    fn test_update_risk_parameters_success() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32));
        unwrap_ok(client.update_risk_parameters(&borrower, &2000_i128, &400_u32, &85_u32));

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.credit_limit, 2000);
        assert_eq!(credit_line.interest_rate_bps, 400);
        assert_eq!(credit_line.risk_score, 85);
    }

    /// `update_risk_parameters` panics (auth failure) when called without admin auth.
    #[test]
    #[should_panic]
    fn test_update_risk_parameters_unauthorized_caller() {
        let env = Env::default();
        // Do NOT use mock_all_auths: admin.require_auth() will fail.
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client
            .open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32)
            .unwrap();
        client
            .update_risk_parameters(&borrower, &2000_i128, &400_u32, &85_u32)
            .unwrap();
    }

    /// `update_risk_parameters` returns `CreditLineNotFound` for an unknown borrower.
    #[test]
    fn test_update_risk_parameters_nonexistent_line() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        let err = client
            .try_update_risk_parameters(&borrower, &1000_i128, &300_u32, &70_u32)
            .expect_err("should fail");
        assert_eq!(
            err.unwrap(),
            CreditError::CreditLineNotFound,
            "expected CreditLineNotFound (code 2)"
        );
    }

    /// `update_risk_parameters` returns `InvalidCreditLimit` when new limit < utilized.
    #[test]
    fn test_update_risk_parameters_credit_limit_below_utilized() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32));
        unwrap_ok(client.draw_credit(&borrower, &500_i128));

        let err = client
            .try_update_risk_parameters(&borrower, &300_i128, &300_u32, &70_u32)
            .expect_err("should fail");
        assert_eq!(
            err.unwrap(),
            CreditError::InvalidCreditLimit,
            "expected InvalidCreditLimit (code 7)"
        );
    }

    /// `update_risk_parameters` returns `InvalidCreditLimit` for a negative limit.
    #[test]
    fn test_update_risk_parameters_negative_credit_limit() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32));

        let err = client
            .try_update_risk_parameters(&borrower, &(-1_i128), &300_u32, &70_u32)
            .expect_err("should fail");
        assert_eq!(
            err.unwrap(),
            CreditError::InvalidCreditLimit,
            "expected InvalidCreditLimit (code 7)"
        );
    }

    /// `update_risk_parameters` returns `InterestRateExceedsMax` when bps > 10 000.
    #[test]
    fn test_update_risk_parameters_interest_rate_exceeds_max() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32));

        let err = client
            .try_update_risk_parameters(&borrower, &1000_i128, &10001_u32, &70_u32)
            .expect_err("should fail");
        assert_eq!(
            err.unwrap(),
            CreditError::InterestRateExceedsMax,
            "expected InterestRateExceedsMax (code 8)"
        );
    }

    /// `update_risk_parameters` returns `RiskScoreExceedsMax` when score > 100.
    #[test]
    fn test_update_risk_parameters_risk_score_exceeds_max() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32));

        let err = client
            .try_update_risk_parameters(&borrower, &1000_i128, &300_u32, &101_u32)
            .expect_err("should fail");
        assert_eq!(
            err.unwrap(),
            CreditError::RiskScoreExceedsMax,
            "expected RiskScoreExceedsMax (code 9)"
        );
    }

    /// Boundary values (10 000 bps, score 100) are accepted.
    #[test]
    fn test_update_risk_parameters_at_boundaries() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32));
        unwrap_ok(client.update_risk_parameters(&borrower, &1000_i128, &10000_u32, &100_u32));

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.interest_rate_bps, 10000);
        assert_eq!(credit_line.risk_score, 100);
    }

    // ── repay_credit ──────────────────────────────────────────────────────────

    /// `repay_credit` reduces `utilized_amount` and emits exactly one event.
    #[test]
    fn test_repay_credit_reduces_utilized_and_emits_event() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32));
        unwrap_ok(client.draw_credit(&borrower, &500_i128));

        let events_before = env.events().all().len();
        unwrap_ok(client.repay_credit(&borrower, &200_i128));
        let events_after = env.events().all().len();

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.utilized_amount, 300);
        assert_eq!(
            events_after,
            events_before + 1,
            "repay_credit must emit exactly one RepaymentEvent"
        );
    }

    /// `repay_credit` saturates at zero (no negative utilization).
    #[test]
    fn test_repay_credit_saturates_at_zero() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32));
        unwrap_ok(client.draw_credit(&borrower, &100_i128));
        unwrap_ok(client.repay_credit(&borrower, &500_i128));

        let credit_line = client.get_credit_line(&borrower).unwrap();
        assert_eq!(credit_line.utilized_amount, 0);
    }

    /// `repay_credit` returns `InvalidAmount` for a zero amount.
    #[test]
    fn test_repay_credit_rejects_non_positive_amount() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32));

        let err = client
            .try_repay_credit(&borrower, &0_i128)
            .expect_err("should fail");
        assert_eq!(
            err.unwrap(),
            CreditError::InvalidAmount,
            "expected InvalidAmount (code 3)"
        );
    }

    /// `repay_credit` returns `CreditLineNotFound` for an unknown borrower.
    #[test]
    fn test_repay_credit_nonexistent_line() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        let err = client
            .try_repay_credit(&borrower, &100_i128)
            .expect_err("should fail");
        assert_eq!(
            err.unwrap(),
            CreditError::CreditLineNotFound,
            "expected CreditLineNotFound (code 2)"
        );
    }

    /// `repay_credit` returns `CreditLineClosed` on a closed line.
    #[test]
    fn test_repay_credit_rejected_when_closed() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32));
        unwrap_ok(client.close_credit_line(&borrower, &admin));

        let err = client
            .try_repay_credit(&borrower, &100_i128)
            .expect_err("should fail");
        assert_eq!(
            err.unwrap(),
            CreditError::CreditLineClosed,
            "expected CreditLineClosed (code 5)"
        );
    }

    // ── Admin-only: unauthorized callers ──────────────────────────────────────

    /// `suspend_credit_line` panics (auth failure) when called without admin auth.
    #[test]
    #[should_panic]
    fn test_suspend_credit_line_unauthorized() {
        let env = Env::default();
        // Do NOT use mock_all_auths.
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client
            .open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32)
            .unwrap();
        client.suspend_credit_line(&borrower).unwrap();
    }

    /// `default_credit_line` panics (auth failure) when called without admin auth.
    #[test]
    #[should_panic]
    fn test_default_credit_line_unauthorized() {
        let env = Env::default();
        // Do NOT use mock_all_auths.
        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        client
            .open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32)
            .unwrap();
        client.default_credit_line(&borrower).unwrap();
    }

    // ── Reentrancy guard ──────────────────────────────────────────────────────
    //
    // We cannot simulate a token callback in unit tests without a mock contract.
    // The guard is exercised indirectly: draw_credit and repay_credit set/clear
    // it.  These tests verify that the guard is properly cleared after each
    // successful call so that subsequent calls succeed.

    /// Guard is cleared after a successful `draw_credit`.
    #[test]
    fn test_reentrancy_guard_cleared_after_draw() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32));
        unwrap_ok(client.draw_credit(&borrower, &100_i128));
        unwrap_ok(client.draw_credit(&borrower, &100_i128));
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            200
        );
    }

    /// Guard is cleared after a successful `repay_credit`.
    #[test]
    fn test_reentrancy_guard_cleared_after_repay() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        unwrap_ok(client.open_credit_line(&borrower, &1000_i128, &300_u32, &70_u32));
        unwrap_ok(client.draw_credit(&borrower, &200_i128));
        unwrap_ok(client.repay_credit(&borrower, &50_i128));
        unwrap_ok(client.repay_credit(&borrower, &50_i128));
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            100
        );
    }

    // ── open_credit_line validation ───────────────────────────────────────────

    /// `open_credit_line` returns `InvalidCreditLimit` for a negative limit.
    #[test]
    fn test_open_credit_line_negative_limit() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        let err = client
            .try_open_credit_line(&borrower, &(-1_i128), &300_u32, &70_u32)
            .expect_err("should fail");
        assert_eq!(
            err.unwrap(),
            CreditError::InvalidCreditLimit,
            "expected InvalidCreditLimit (code 7)"
        );
    }

    /// `open_credit_line` returns `InterestRateExceedsMax` when bps > 10 000.
    #[test]
    fn test_open_credit_line_interest_rate_exceeds_max() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        let err = client
            .try_open_credit_line(&borrower, &1000_i128, &10001_u32, &70_u32)
            .expect_err("should fail");
        assert_eq!(
            err.unwrap(),
            CreditError::InterestRateExceedsMax,
            "expected InterestRateExceedsMax (code 8)"
        );
    }

    /// `open_credit_line` returns `RiskScoreExceedsMax` when score > 100.
    #[test]
    fn test_open_credit_line_risk_score_exceeds_max() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let borrower = Address::generate(&env);

        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(&env, &contract_id);

        client.init(&admin);
        let err = client
            .try_open_credit_line(&borrower, &1000_i128, &300_u32, &101_u32)
            .expect_err("should fail");
        assert_eq!(
            err.unwrap(),
            CreditError::RiskScoreExceedsMax,
            "expected RiskScoreExceedsMax (code 9)"
        );
    }
}
