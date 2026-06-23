// SPDX-License-Identifier: MIT
//
// Integration tests for issue #343 — external default oracle for liquidation
// valuation.
//
// Each test registers a mock Soroban oracle contract and the production
// credit contract in the test environment, then exercises the credit contract's
// `set_default_oracle` / `get_default_oracle` / `clear_default_oracle`
// entrypoints and the cross-contract invocation path inside
// `settle_default_liquidation`.

use soroban_sdk::{
    contract, contractimpl, symbol_short, testutils::Address as _, testutils::Events as _,
    testutils::Ledger, Address, Env, Symbol, TryFromVal,
};

use creditra_credit::types::CreditLineData;
use creditra_credit::types::CreditStatus;
use creditra_credit::CreditClient;

// ── Mock default-oracle contract ─────────────────────────────────────────────────
//
// The mock contract exposes the `latest_price() -> (i128, u64)` view that the
// production credit contract invokes over `env.invoke_contract`. The test
// scaffolding sets the price/timestamp via `set_quote` and `init`.

#[contract]
pub struct MockDefaultOracle;

#[contractimpl]
impl MockDefaultOracle {
    pub fn init(env: Env, initial_price: i128, initial_ts: u64) {
        env.storage()
            .instance()
            .set(&symbol_short!("price"), &initial_price);
        env.storage()
            .instance()
            .set(&symbol_short!("ts"), &initial_ts);
    }

    pub fn set_quote(env: Env, price: i128, ts: u64) {
        env.storage().instance().set(&symbol_short!("price"), &price);
        env.storage().instance().set(&symbol_short!("ts"), &ts);
    }

    /// Production-callable view. Returns the latest `(price, timestamp)` pair.
    pub fn latest_price(env: Env) -> (i128, u64) {
        let price: i128 = env
            .storage()
            .instance()
            .get(&symbol_short!("price"))
            .unwrap_or(0);
        let ts: u64 = env
            .storage()
            .instance()
            .get(&symbol_short!("ts"))
            .unwrap_or(0);
        (price, ts)
    }
}

// ── Test fixtures ───────────────────────────────────────────────────────────────────────

struct Fixture {
    contract_id: Address,
    client: CreditClient<'static>,
    oracle_addr: Address,
    oracle: MockDefaultOracleClient<'static>,
    admin: Address,
    borrower: Address,
}

fn fixture(env: &Env) -> Fixture {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(creditra_credit::Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let oracle_addr = env.register(MockDefaultOracle, ());
    let oracle = MockDefaultOracleClient::new(env, &oracle_addr);

    Fixture {
        contract_id,
        client,
        oracle_addr,
        oracle,
        admin,
        borrower,
    }
}

/// Seed a credit line directly into the Defaulted status with the requested
/// `utilized_amount`. Uses internal storage writes from the same contract
/// context — fixture picks allowed to bypass the production draw flow
/// because admin-gated defaulting does not require a liquidity token.
fn seed_defaulted_line(env: &Env, fx: &Fixture, utilized: i128) {
    env.as_contract(&fx.contract_id, || {
        let line = CreditLineData {
            borrower: fx.borrower.clone(),
            credit_limit: 10_000,
            utilized_amount: utilized,
            interest_rate_bps: 300,
            risk_score: 70,
            status: CreditStatus::Defaulted,
            last_rate_update_ts: 0,
            accrued_interest: 0,
            last_accrual_ts: env.ledger().timestamp(),
            suspension_ts: 0,
        };
        env.storage().persistent().set(&fx.borrower, &line);
    });
}

// ── Tests: admin-authorized oracle configuration entrypoints ────────────────────────────

#[test]
fn set_default_oracle_stores_config() {
    let env = Env::default();
    let fx = fixture(&env);
    fx.oracle.init(&100_i128, &1_000_u64);

    fx.client.set_default_oracle(&fx.oracle_addr, &3_600_u64);

    let cfg = fx.client.get_default_oracle().unwrap();
    assert_eq!(cfg.oracle_address, fx.oracle_addr);
    assert_eq!(cfg.max_price_age_seconds, 3_600);
}

#[test]
fn set_default_oracle_emits_event_with_correct_topic() {
    let env = Env::default();
    let fx = fixture(&env);
    fx.oracle.init(&100_i128, &1_000_u64);

    fx.client.set_default_oracle(&fx.oracle_addr, &60_u64);

    let events = env.events().all();
    let (_, topics, _) = events.last().unwrap();
    let topic0: Symbol = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
    let topic1: Symbol = Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap();
    assert_eq!(topic0, symbol_short!("credit"));
    assert_eq!(topic1, Symbol::new(&env, "oracle_cfg"));
}

#[test]
fn clear_default_oracle_removes_config() {
    let env = Env::default();
    let fx = fixture(&env);
    fx.oracle.init(&100_i128, &1_000_u64);

    fx.client.set_default_oracle(&fx.oracle_addr, &60_u64);
    assert!(fx.client.get_default_oracle().is_some());

    fx.client.clear_default_oracle();
    assert!(fx.client.get_default_oracle().is_none());
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn set_default_oracle_rejects_zero_age_bound() {
    let env = Env::default();
    let fx = fixture(&env);

    fx.client.set_default_oracle(&fx.oracle_addr, &0_u64);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn set_default_oracle_rejects_one_year_plus_one_age_bound() {
    let env = Env::default();
    let fx = fixture(&env);

    // 365 days + 1 second — out of bounds.
    let too_long = (365 * 24 * 60 * 60) + 1;
    fx.client.set_default_oracle(&fx.oracle_addr, &too_long);
}

#[test]
fn set_default_oracle_accepts_one_year_age_bound() {
    let env = Env::default();
    let fx = fixture(&env);

    // 365 days exactly — accepted.
    let one_year = 365 * 24 * 60 * 60;
    fx.client.set_default_oracle(&fx.oracle_addr, &one_year);
    let cfg = fx.client.get_default_oracle().unwrap();
    assert_eq!(cfg.max_price_age_seconds, one_year);
}

#[test]
fn get_default_oracle_returns_configured_config() {
    let env = Env::default();
    let fx = fixture(&env);
    fx.oracle.init(&100_i128, &1_000_u64);

    fx.client.set_default_oracle(&fx.oracle_addr, &60_u64);
    let cfg = fx.client.get_default_oracle().unwrap();
    assert_eq!(cfg.oracle_address, fx.oracle_addr);
    assert_eq!(cfg.max_price_age_seconds, 60);
}

// ── Tests: settle_default_liquidation oracle-driven path ────────────────────────────────

#[test]
fn settle_happy_path_at_parity_price() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = 5_000);
    let fx = fixture(&env);
    // Parity: 1e9 == 1 unit of base token worth 1 unit of base token.
    fx.oracle.init(&1_000_000_000_i128, &5_000_u64);
    fx.client.set_default_oracle(&fx.oracle_addr, &60_u64);
    seed_defaulted_line(&env, &fx, 1_000);

    fx.client
        .settle_default_liquidation(&fx.borrower, &800_i128, &symbol_short!("set1"));

    let line = fx.client.get_credit_line(&fx.borrower).unwrap();
    assert_eq!(line.utilized_amount, 200);
    assert_ne!(line.status, CreditStatus::Closed);
}

#[test]
fn settle_happy_path_closes_line_at_full_recovery() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = 5_000);
    let fx = fixture(&env);
    fx.oracle.init(&1_000_000_000_i128, &5_000_u64);
    fx.client.set_default_oracle(&fx.oracle_addr, &60_u64);
    seed_defaulted_line(&env, &fx, 1_000);

    fx.client
        .settle_default_liquidation(&fx.borrower, &1_000_i128, &symbol_short!("set1"));

    let line = fx.client.get_credit_line(&fx.borrower).unwrap();
    assert_eq!(line.utilized_amount, 0);
    assert_eq!(line.status, CreditStatus::Closed);
}

#[test]
#[should_panic(expected = "Error(Contract, #30)")] // MissingOracle
fn settle_reverts_when_no_oracle_configured() {
    let env = Env::default();
    let fx = fixture(&env);
    seed_defaulted_line(&env, &fx, 500);

    // No `set_default_oracle` was called.
    fx.client
        .settle_default_liquidation(&fx.borrower, &100_i128, &symbol_short!("set1"));
}

#[test]
#[should_panic(expected = "Error(Contract, #31)")] // OraclePriceStale
fn settle_reverts_when_oracle_price_is_stale() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = 100_000);
    let fx = fixture(&env);
    fx.oracle.init(&1_000_000_000_i128, &1_000_u64); // age = 99_000s
    fx.client.set_default_oracle(&fx.oracle_addr, &60_u64);
    seed_defaulted_line(&env, &fx, 1_000);

    fx.client
        .settle_default_liquidation(&fx.borrower, &100_i128, &symbol_short!("set1"));
}

#[test]
#[should_panic(expected = "Error(Contract, #32)")] // OraclePriceInvalid
fn settle_reverts_when_oracle_price_is_zero() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = 5_000);
    let fx = fixture(&env);
    fx.oracle.init(&0_i128, &5_000_u64);
    fx.client.set_default_oracle(&fx.oracle_addr, &60_u64);
    seed_defaulted_line(&env, &fx, 500);

    fx.client
        .settle_default_liquidation(&fx.borrower, &100_i128, &symbol_short!("set1"));
}

#[test]
#[should_panic(expected = "Error(Contract, #32)")] // OraclePriceInvalid
fn settle_reverts_when_oracle_price_is_negative() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = 5_000);
    let fx = fixture(&env);
    fx.oracle.init(&-1_i128, &5_000_u64);
    fx.client.set_default_oracle(&fx.oracle_addr, &60_u64);
    seed_defaulted_line(&env, &fx, 500);

    fx.client
        .settle_default_liquidation(&fx.borrower, &100_i128, &symbol_short!("set1"));
}

#[test]
#[should_panic(expected = "Error(Contract, #32)")] // OraclePriceInvalid
fn settle_reverts_when_oracle_timestamp_is_future() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = 5_000);
    let fx = fixture(&env);
    fx.oracle.init(&1_000_000_000_i128, &6_000_u64); // ts > now
    fx.client.set_default_oracle(&fx.oracle_addr, &60_u64);
    seed_defaulted_line(&env, &fx, 1_000);

    fx.client
        .settle_default_liquidation(&fx.borrower, &100_i128, &symbol_short!("set1"));
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")] // OracleRecoveryExceedsBound
fn settle_reverts_when_recovery_exceeds_oracle_bound() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = 5_000);
    let fx = fixture(&env);
    // 0.5x parity: oracle-derived upper bound on a 1_000-utilized position is 500.
    fx.oracle.init(&500_000_000_i128, &5_000_u64);
    fx.client.set_default_oracle(&fx.oracle_addr, &60_u64);
    seed_defaulted_line(&env, &fx, 1_000);

    // 800 > 500 bound.
    fx.client
        .settle_default_liquidation(&fx.borrower, &800_i128, &symbol_short!("set1"));
}

#[test]
fn settle_accepts_oracle_bound_at_half_price() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = 5_000);
    let fx = fixture(&env);
    fx.oracle.init(&500_000_000_i128, &5_000_u64);
    fx.client.set_default_oracle(&fx.oracle_addr, &60_u64);
    seed_defaulted_line(&env, &fx, 1_000);

    // 500 == 500 bound (floor(price * utilized / 1e9)).
    fx.client
        .settle_default_liquidation(&fx.borrower, &500_i128, &symbol_short!("set1"));
    let line = fx.client.get_credit_line(&fx.borrower).unwrap();
    assert_eq!(line.utilized_amount, 500);
}

#[test]
fn stale_age_at_exact_boundary_succeeds() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = 1_000);
    let fx = fixture(&env);
    fx.oracle.init(&1_000_000_000_i128, &940_u64); // age = 60 == bound
    fx.client.set_default_oracle(&fx.oracle_addr, &60_u64);
    seed_defaulted_line(&env, &fx, 1_000);

    fx.client
        .settle_default_liquidation(&fx.borrower, &100_i128, &symbol_short!("set1"));
    let line = fx.client.get_credit_line(&fx.borrower).unwrap();
    assert_eq!(line.utilized_amount, 900);
}

#[test]
fn stale_age_just_over_boundary_reverts() {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = 1_000);
    let fx = fixture(&env);
    fx.oracle.init(&1_000_000_000_i128, &939_u64); // age = 61 > 60
    fx.client.set_default_oracle(&fx.oracle_addr, &60_u64);
    seed_defaulted_line(&env, &fx, 1_000);

    let _ = fx.client.try_settle_default_liquidation(
        &fx.borrower,
        &100_i128,
        &symbol_short!("set1"),
    );
    // The transaction panics during execution; the credit line is unchanged.
    let line = fx.client.get_credit_line(&fx.borrower).unwrap();
    assert_eq!(line.utilized_amount, 1_000); // unchanged
}

// ── Sanity check: error discriminants appended at the end of the table ──────────────────

#[test]
fn oracle_error_discriminants_match_documented_table() {
    use creditra_credit::types::ContractError;

    assert_eq!(ContractError::MissingOracle as u32, 30);
    assert_eq!(ContractError::OraclePriceStale as u32, 31);
    assert_eq!(ContractError::OraclePriceInvalid as u32, 32);
    assert_eq!(ContractError::OracleRecoveryExceedsBound as u32, 33);
}
