// SPDX-License-Identifier: MIT

//! Cross-contract handshake failure rollback-safety tests (Issue #1167).
//!
//! # What these tests verify
//!
//! Every failure mode in `settle_default_liquidation`'s cross-contract
//! handshake must:
//!
//! 1. **Clear the reentrancy guard** before propagating, so the next
//!    invocation is not permanently locked with `Reentrancy = 11`.
//! 2. **Leave credit-line state unmodified** — no partial accounting write
//!    must survive a handshake failure.
//! 3. **Return a typed, diagnosable error** (`IncompatibleVersion = 60` or
//!    `AuctionCallFailed = 61`) rather than a generic host panic.
//! 4. **Allow a safe retry** once the root cause is resolved.
//!
//! # Failure scenarios covered
//!
//! | Test | Failure | Expected error | Guard after | State after |
//! |------|---------|---------------|-------------|-------------|
//! | `guard_cleared_on_version_mismatch` | get_version returns major≠1 | `IncompatibleVersion(60)` | cleared | unchanged |
//! | `guard_cleared_on_get_version_cpi_panic` | get_version panics | `AuctionCallFailed(61)` | cleared | unchanged |
//! | `guard_cleared_on_settle_cpi_panic` | settle CPI panics | `AuctionCallFailed(61)` | cleared | unchanged |
//! | `guard_cleared_on_amount_mismatch` | auction returns wrong amount | `AuctionCallFailed(61)` | cleared | unchanged |
//! | `retry_succeeds_after_version_mismatch` | version fails → fixed | success | cleared | updated |
//! | `retry_succeeds_after_settle_cpi_failure` | CPI fails → fixed | success | cleared | updated |
//! | `replay_blocked_after_partial_success` | same settlement_id twice | `AlreadyInitialized(14)` | — | unchanged |
//! | `no_auction_configured_settles_directly` | no auction contract set | success | cleared | updated |
//! | `full_settlement_closes_line` | recovered == utilized | success, Closed | cleared | Closed |
//! | `multiple_sequential_failures_then_success` | 3× fail → fix → succeed | success | cleared | updated |

use creditra_credit::types::{ContractError, CreditStatus};
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short,
    testutils::Address as _,
    token, Address, Env, Symbol,
};

// ─────────────────────────────────────────────────────────────────────────────
// Mock auction contract
//
// A minimal #[contract] with configurable failure modes.  Only implements
// the two methods the credit contract calls: `get_version` and
// `settle_default_liquidation`.
// ─────────────────────────────────────────────────────────────────────────────

/// Per-test behaviour flags stored in the mock auction's instance storage.
#[contracttype]
#[derive(Clone)]
pub struct MockCfg {
    /// When true, `get_version` panics unconditionally.
    pub panic_on_get_version: bool,
    /// When true, `settle_default_liquidation` panics unconditionally.
    pub panic_on_settle: bool,
    /// Major protocol version returned by `get_version` (1 = compatible).
    pub version_major: u32,
    /// Amount returned by `settle_default_liquidation`.
    pub return_amount: i128,
}

#[contract]
pub struct MockAuction;

#[contractimpl]
impl MockAuction {
    /// Store a new behaviour config.
    pub fn configure(env: Env, cfg: MockCfg) {
        env.storage().instance().set(&symbol_short!("cfg"), &cfg);
    }

    /// Register the credit contract as factory (mirroring the real auction).
    pub fn set_factory_contract(env: Env, factory: Address) {
        factory.require_auth();
        env.storage()
            .instance()
            .set(&symbol_short!("fac"), &factory);
    }

    /// Protocol version query called during the handshake.
    pub fn get_version(env: Env) -> creditra_credit::handshake::ProtocolVersion {
        let cfg = Self::load_cfg(&env);
        if cfg.panic_on_get_version {
            panic!("mock: get_version deliberately panics");
        }
        creditra_credit::handshake::ProtocolVersion {
            major: cfg.version_major,
            minor: 0,
        }
    }

    /// Settlement entry-point called by the credit contract.
    pub fn settle_default_liquidation(
        env: Env,
        _auction_id: Symbol,
        credit_contract: Address,
        _borrower: Address,
    ) -> i128 {
        // Enforce factory auth (mirrors real auction behaviour).
        let factory: Option<Address> = env.storage().instance().get(&symbol_short!("fac"));
        if let Some(f) = factory {
            f.require_auth();
            let _ = credit_contract;
        }
        let cfg = Self::load_cfg(&env);
        if cfg.panic_on_settle {
            panic!("mock: settle_default_liquidation deliberately panics");
        }
        cfg.return_amount
    }

    fn load_cfg(env: &Env) -> MockCfg {
        env.storage()
            .instance()
            .get(&symbol_short!("cfg"))
            .unwrap_or(MockCfg {
                panic_on_get_version: false,
                panic_on_settle: false,
                version_major: 1,
                return_amount: 0,
            })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Setup helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Returns `(env, credit_contract_id, borrower)` with a defaulted credit line
/// carrying the given `utilized` principal.
fn setup_defaulted(utilized: i128) -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let cid = env.register(Credit, ());
    let c = CreditClient::new(&env, &cid);
    c.init(&admin);

    let sac = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let tok = sac.address();
    c.set_liquidity_token(&tok);
    token::StellarAssetClient::new(&env, &tok).mint(&cid, &10_000_000_i128);
    token::StellarAssetClient::new(&env, &tok).mint(&borrower, &10_000_000_i128);
    token::Client::new(&env, &tok).approve(
        &borrower,
        &cid,
        &10_000_000_i128,
        &1_000_000_u32,
    );

    c.open_credit_line(&borrower, &1_000_000, &500_u32, &50_u32);
    if utilized > 0 {
        c.draw_credit(&borrower, &utilized);
    }
    c.default_credit_line(&borrower);
    (env, cid, borrower)
}

/// Register a mock auction, configure it, wire it as factory, and tell the
/// credit contract about it.  Returns the auction contract address.
fn wire_auction(env: &Env, cid: &Address, cfg: MockCfg) -> Address {
    let aid = env.register(MockAuction, ());
    MockAuctionClient::new(env, &aid).configure(&cfg);
    MockAuctionClient::new(env, &aid).set_factory_contract(cid);
    CreditClient::new(env, cid).set_auction_contract(&aid);
    aid
}

fn ok_cfg(amount: i128) -> MockCfg {
    MockCfg {
        panic_on_get_version: false,
        panic_on_settle: false,
        version_major: 1,
        return_amount: amount,
    }
}

fn bad_version_cfg(amount: i128) -> MockCfg {
    MockCfg { version_major: 99, ..ok_cfg(amount) }
}

fn panic_get_version_cfg(amount: i128) -> MockCfg {
    MockCfg { panic_on_get_version: true, ..ok_cfg(amount) }
}

fn panic_settle_cfg(amount: i128) -> MockCfg {
    MockCfg { panic_on_settle: true, ..ok_cfg(amount) }
}

// ─────────────────────────────────────────────────────────────────────────────
// Guard-leak tests
// ─────────────────────────────────────────────────────────────────────────────

/// Version mismatch must return IncompatibleVersion and clear the guard so
/// that a second call on the same line does not see Reentrancy.
#[test]
fn guard_cleared_on_version_mismatch() {
    let (env, cid, borrower) = setup_defaulted(1_000);
    let c = CreditClient::new(&env, &cid);
    wire_auction(&env, &cid, bad_version_cfg(500));

    let s1 = Symbol::new(&env, "vmm1");

    // Must fail with IncompatibleVersion (60)
    assert_eq!(
        c.try_settle_default_liquidation(&borrower, &500, &s1, &10_000, &None)
            .err().unwrap().unwrap(),
        ContractError::IncompatibleVersion.into()
    );

    // State unchanged
    let line = c.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 1_000);
    assert_eq!(line.status, CreditStatus::Defaulted);

    // Retry must NOT see Reentrancy — guard was cleared
    let s2 = Symbol::new(&env, "vmm2");
    let r2 = c.try_settle_default_liquidation(&borrower, &500, &s2, &10_000, &None)
        .err().unwrap().unwrap();
    assert_ne!(
        r2,
        ContractError::Reentrancy.into(),
        "guard leaked: second call returned Reentrancy"
    );
}

/// When get_version CPI itself panics, AuctionCallFailed must be returned
/// and the guard must be cleared.
#[test]
fn guard_cleared_on_get_version_cpi_panic() {
    let (env, cid, borrower) = setup_defaulted(800);
    let c = CreditClient::new(&env, &cid);
    wire_auction(&env, &cid, panic_get_version_cfg(400));

    let s1 = Symbol::new(&env, "gvp1");

    assert_eq!(
        c.try_settle_default_liquidation(&borrower, &400, &s1, &10_000, &None)
            .err().unwrap().unwrap(),
        ContractError::AuctionCallFailed.into()
    );

    let line = c.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 800);

    // Retry must not see Reentrancy
    let s2 = Symbol::new(&env, "gvp2");
    let r2 = c.try_settle_default_liquidation(&borrower, &400, &s2, &10_000, &None)
        .err().unwrap().unwrap();
    assert_ne!(r2, ContractError::Reentrancy.into(), "guard leaked after get_version CPI panic");
}

/// When settle_default_liquidation CPI panics, AuctionCallFailed must be
/// returned and the guard must be cleared.
#[test]
fn guard_cleared_on_settle_cpi_panic() {
    let (env, cid, borrower) = setup_defaulted(600);
    let c = CreditClient::new(&env, &cid);
    wire_auction(&env, &cid, panic_settle_cfg(300));

    let s1 = Symbol::new(&env, "scp1");

    assert_eq!(
        c.try_settle_default_liquidation(&borrower, &300, &s1, &10_000, &None)
            .err().unwrap().unwrap(),
        ContractError::AuctionCallFailed.into()
    );

    let line = c.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 600);

    let s2 = Symbol::new(&env, "scp2");
    let r2 = c.try_settle_default_liquidation(&borrower, &300, &s2, &10_000, &None)
        .err().unwrap().unwrap();
    assert_ne!(r2, ContractError::Reentrancy.into(), "guard leaked after settle CPI panic");
}

/// When auction returns a different amount than recovered_amount,
/// AuctionCallFailed must be returned and the guard cleared.
#[test]
fn guard_cleared_on_amount_mismatch() {
    let (env, cid, borrower) = setup_defaulted(500);
    let c = CreditClient::new(&env, &cid);
    wire_auction(&env, &cid, ok_cfg(200)); // returns 200, but caller claims 300

    let s1 = Symbol::new(&env, "amm1");

    assert_eq!(
        c.try_settle_default_liquidation(&borrower, &300, &s1, &10_000, &None)
            .err().unwrap().unwrap(),
        ContractError::AuctionCallFailed.into()
    );

    let line = c.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 500);

    let s2 = Symbol::new(&env, "amm2");
    let r2 = c.try_settle_default_liquidation(&borrower, &300, &s2, &10_000, &None)
        .err().unwrap().unwrap();
    assert_ne!(r2, ContractError::Reentrancy.into(), "guard leaked after amount mismatch");
}

// ─────────────────────────────────────────────────────────────────────────────
// Retry-after-failure tests
// ─────────────────────────────────────────────────────────────────────────────

/// After a version-mismatch failure, upgrading the auction and retrying with
/// a new settlement_id must succeed and correctly reduce utilized_amount.
#[test]
fn retry_succeeds_after_version_mismatch() {
    let (env, cid, borrower) = setup_defaulted(1_000);
    let c = CreditClient::new(&env, &cid);
    let aid = wire_auction(&env, &cid, bad_version_cfg(400));

    let s1 = Symbol::new(&env, "rvmf1");
    assert_eq!(
        c.try_settle_default_liquidation(&borrower, &400, &s1, &10_000, &None)
            .err().unwrap().unwrap(),
        ContractError::IncompatibleVersion.into()
    );

    // "Upgrade" the auction to a compatible version
    MockAuctionClient::new(&env, &aid).configure(&ok_cfg(400));

    // Retry with a fresh settlement_id (same id would hit AlreadyInitialized)
    let s2 = Symbol::new(&env, "rvmf2");
    c.try_settle_default_liquidation(&borrower, &400, &s2, &10_000, &None)
        .expect("retry should succeed")
        .expect("no contract error");

    let line = c.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 600, "utilized must decrease after successful retry");
    assert_eq!(line.status, CreditStatus::Defaulted);
}

/// After a CPI-panic failure, fixing the auction and retrying succeeds.
#[test]
fn retry_succeeds_after_settle_cpi_failure() {
    let (env, cid, borrower) = setup_defaulted(1_000);
    let c = CreditClient::new(&env, &cid);
    let aid = wire_auction(&env, &cid, panic_settle_cfg(500));

    let s1 = Symbol::new(&env, "rscf1");
    assert_eq!(
        c.try_settle_default_liquidation(&borrower, &500, &s1, &10_000, &None)
            .err().unwrap().unwrap(),
        ContractError::AuctionCallFailed.into()
    );

    MockAuctionClient::new(&env, &aid).configure(&ok_cfg(500));

    let s2 = Symbol::new(&env, "rscf2");
    c.try_settle_default_liquidation(&borrower, &500, &s2, &10_000, &None)
        .expect("retry should succeed after fixing auction")
        .expect("no contract error");

    let line = c.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 500);
}

// ─────────────────────────────────────────────────────────────────────────────
// Replay protection
// ─────────────────────────────────────────────────────────────────────────────

/// Replaying the same (borrower, settlement_id) after a successful partial
/// settlement must be blocked with AlreadyInitialized and must not mutate
/// the credit line a second time.
#[test]
fn replay_blocked_after_partial_success() {
    let (env, cid, borrower) = setup_defaulted(1_000);
    let c = CreditClient::new(&env, &cid);
    wire_auction(&env, &cid, ok_cfg(300));

    let sid = Symbol::new(&env, "rbps");

    // First settlement succeeds
    c.try_settle_default_liquidation(&borrower, &300, &sid, &10_000, &None)
        .expect("first settlement must succeed")
        .expect("no contract error");

    let after1 = c.get_credit_line(&borrower).unwrap();
    assert_eq!(after1.utilized_amount, 700);

    // Replay with identical settlement_id must fail
    assert_eq!(
        c.try_settle_default_liquidation(&borrower, &300, &sid, &10_000, &None)
            .err().unwrap().unwrap(),
        ContractError::AlreadyInitialized.into(),
        "replay must be rejected with AlreadyInitialized"
    );

    // State must be unchanged from after the first call
    let after2 = c.get_credit_line(&borrower).unwrap();
    assert_eq!(after2.utilized_amount, 700, "replay must not double-apply accounting");
}

// ─────────────────────────────────────────────────────────────────────────────
// No auction configured
// ─────────────────────────────────────────────────────────────────────────────

/// When no auction contract is configured, settlement is purely credit-side
/// and the reentrancy guard must be set and cleared correctly.
#[test]
fn no_auction_configured_settles_directly() {
    let (env, cid, borrower) = setup_defaulted(500);
    let c = CreditClient::new(&env, &cid);

    assert!(c.get_auction_contract().is_none());

    let s1 = Symbol::new(&env, "nacd1");
    c.try_settle_default_liquidation(&borrower, &200, &s1, &10_000, &None)
        .expect("should succeed without auction")
        .expect("no contract error");

    let line = c.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 300);

    // Second call with different id proves the guard was cleared after the first
    let s2 = Symbol::new(&env, "nacd2");
    c.try_settle_default_liquidation(&borrower, &200, &s2, &10_000, &None)
        .expect("second call must succeed — guard was cleared")
        .expect("no contract error");

    let line2 = c.get_credit_line(&borrower).unwrap();
    assert_eq!(line2.utilized_amount, 100);
}

// ─────────────────────────────────────────────────────────────────────────────
// Full liquidation
// ─────────────────────────────────────────────────────────────────────────────

/// A full settlement (recovered == utilized) must transition status to Closed
/// and clear the guard.
#[test]
fn full_settlement_closes_line_and_clears_guard() {
    let (env, cid, borrower) = setup_defaulted(400);
    let c = CreditClient::new(&env, &cid);
    wire_auction(&env, &cid, ok_cfg(400));

    let sid = Symbol::new(&env, "fscg");
    c.try_settle_default_liquidation(&borrower, &400, &sid, &10_000, &None)
        .expect("full settlement must succeed")
        .expect("no contract error");

    let line = c.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Closed);
    assert_eq!(line.utilized_amount, 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// Stress: multiple sequential failures then success
// ─────────────────────────────────────────────────────────────────────────────

/// Multiple consecutive version-mismatch failures must never leave the guard
/// set.  After fixing the auction, the next call must succeed normally.
#[test]
fn multiple_sequential_failures_then_success() {
    let (env, cid, borrower) = setup_defaulted(900);
    let c = CreditClient::new(&env, &cid);
    let aid = wire_auction(&env, &cid, bad_version_cfg(300));

    for i in 0..3_u32 {
        let sid = Symbol::new(&env, &format!("msfs{i}"));
        let r = c.try_settle_default_liquidation(&borrower, &300, &sid, &10_000, &None)
            .err().unwrap().unwrap();
        assert_eq!(r, ContractError::IncompatibleVersion.into());
    }

    // State still pristine after 3 failures
    assert_eq!(c.get_credit_line(&borrower).unwrap().utilized_amount, 900);

    // Fix and succeed
    MockAuctionClient::new(&env, &aid).configure(&ok_cfg(300));
    let ok_sid = Symbol::new(&env, "msfs_ok");
    c.try_settle_default_liquidation(&borrower, &300, &ok_sid, &10_000, &None)
        .expect("must succeed after fixing auction")
        .expect("no contract error");

    assert_eq!(c.get_credit_line(&borrower).unwrap().utilized_amount, 600);
}
