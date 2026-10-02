//! Default-liquidation auction hook tests.
//!
//! Covers the cross-contract settlement hook in `Credit::settle_default_liquidation`:
//!
//! 1. the happy path against the real `gateway-auction` contract, and
//! 2. every rejection path against a *configurable* mock auction contract
//!    (`MockAuction`): recovered-amount mismatch in both directions, a
//!    `get_version` CPI failure, an incompatible major version, and a
//!    settlement CPI failure.
//!
//! The rejection tests exist because the failure contract is stronger than
//! "returns an error": a rejected settlement must be *atomic*. The issue is
//! tracked as #1359 — a mismatch or a broken auction must not consume the
//! `(borrower, settlement_id)` replay marker, must not touch the credit line,
//! must not move the pending-auction counter, and must not tell indexers that
//! a settlement happened. Otherwise a single mismatch bricks the line forever.

use creditra_credit::types::{ContractError, CreditStatus};
use creditra_credit::{Credit, CreditClient};
use gateway_auction::{Auction, AuctionClient, AuctionMode};
use soroban_sdk::testutils::{Address as _, Events as _, Ledger};
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, Address, Env, Symbol, TryFromVal,
};
use std::panic::{catch_unwind, AssertUnwindSafe};

fn setup_auction(
    env: &Env,
    credit_id: &Address,
    auction_id: &Address,
    settlement_id: &Symbol,
    recovered_amount: i128,
) {
    let auction = AuctionClient::new(env, auction_id);
    auction.set_factory_contract(credit_id);

    let start_time = env.ledger().timestamp();
    let end_time = start_time + 1000;
    auction.init_auction(
        settlement_id,
        &AuctionMode::English,
        &start_time,
        &end_time,
        &100_i128,
        &0_u32,
        &None,
        &None,
        // English auction: no Dutch-decay parameters are supplied.
        &None,
        &None,
    );

    let bidder = Address::generate(env);
    auction.place_bid(settlement_id, &bidder, &recovered_amount);

    env.ledger().set_timestamp(end_time);
    auction.close_auction(settlement_id);
}

fn setup_defaulted_line(utilized_amount: i128) -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());

    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &1_000_000_i128);
    token::StellarAssetClient::new(&env, &token_address).mint(&borrower, &1_000_000_i128);
    token::Client::new(&env, &token_address).approve(
        &borrower,
        &contract_id,
        &1_000_000_i128,
        &1_000_000_u32,
    );

    client.open_credit_line(&borrower, &10_000, &300_u32, &60_u32);

    if utilized_amount > 0 {
        // `draw_credit` enforces the minimum collateral ratio (150% when the
        // ratio is unset), so the position needs collateral before it can be
        // drawn — the same shape a real defaulted position has when the
        // auction hook runs. Collateral is denominated in the liquidity token,
        // which the setup above mints to the borrower and approves.
        client.deposit_collateral(&borrower, &(utilized_amount * 2));
        client.draw_credit(&borrower, &utilized_amount);
    }

    client.default_credit_line(&borrower);

    (env, contract_id, borrower)
}

/// Count the `("credit", <event_kind>)` events emitted so far.
///
/// Counting (rather than testing existence) is what lets the failure tests
/// assert that a rejected settlement emitted *nothing*, not merely that some
/// earlier successful call had emitted an event.
fn count_event_topic(env: &Env, event_kind: &str) -> usize {
    let namespace = Symbol::new(env, "credit");
    let kind = Symbol::new(env, event_kind);
    let mut count = 0;

    for (_contract, topics, _data) in env.events().all().iter() {
        // Topics from other contracts (e.g. the SAC) are not all symbols.
        if topics.len() < 2 {
            continue;
        }

        let Ok(t0) = Symbol::try_from_val(env, &topics.get(0).unwrap()) else {
            continue;
        };
        let Ok(t1) = Symbol::try_from_val(env, &topics.get(1).unwrap()) else {
            continue;
        };

        if t0 == namespace && t1 == kind {
            count += 1;
        }
    }

    count
}

fn has_event_topic(env: &Env, event_kind: &str) -> bool {
    count_event_topic(env, event_kind) > 0
}

#[test]
fn default_emits_liquidation_request_event() {
    let (env, _contract_id, _borrower) = setup_defaulted_line(500);

    assert!(has_event_topic(&env, "liq_req"));
}

#[test]
fn settle_partial_default_liquidation_and_block_replay() {
    let (env, contract_id, borrower) = setup_defaulted_line(1_000);
    let client = CreditClient::new(&env, &contract_id);
    let settlement_id = Symbol::new(&env, "auc_001");

    client.settle_default_liquidation(&borrower, &300_i128, &settlement_id, &10_000_u32, &None);
    assert!(has_event_topic(&env, "liq_setl"));

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Defaulted);
    assert_eq!(line.utilized_amount, 700);

    let replay = catch_unwind(AssertUnwindSafe(|| {
        client.settle_default_liquidation(&borrower, &50_i128, &settlement_id, &10_000_u32, &None);
    }));
    assert!(replay.is_err(), "replay settlement should panic");
}

#[test]
fn settle_full_default_liquidation_closes_credit_line() {
    let (env, contract_id, borrower) = setup_defaulted_line(450);
    let client = CreditClient::new(&env, &contract_id);

    client.settle_default_liquidation(
        &borrower,
        &450_i128,
        &Symbol::new(&env, "auc_fin"),
        &10_000_u32,
        &None,
    );
    assert!(has_event_topic(&env, "closed"));
    assert!(has_event_topic(&env, "liq_setl"));

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Closed);
    assert_eq!(line.utilized_amount, 0);
}

#[test]
fn settle_default_liquidation_requires_defaulted_status() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &1_000_000_i128);
    token::StellarAssetClient::new(&env, &token_address).mint(&borrower, &1_000_000_i128);
    token::Client::new(&env, &token_address).approve(
        &borrower,
        &contract_id,
        &1_000_000_i128,
        &1_000_000_u32,
    );

    client.open_credit_line(&borrower, &5_000, &200_u32, &40_u32);
    client.deposit_collateral(&borrower, &1_000_i128);
    client.draw_credit(&borrower, &500_i128);

    let result = catch_unwind(AssertUnwindSafe(|| {
        client.settle_default_liquidation(
            &borrower,
            &100_i128,
            &Symbol::new(&env, "auc_bad"),
            &10_000_u32,
            &None,
        );
    }));

    assert!(result.is_err(), "non-defaulted settlement should panic");
}

// ── Auction contract configuration ─────────────────────────────────────────

#[test]
fn set_and_get_auction_contract_address() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    // Initially no auction contract configured
    assert!(client.get_auction_contract().is_none());

    let auction_addr = Address::generate(&env);
    client.set_auction_contract(&auction_addr);

    assert_eq!(client.get_auction_contract().unwrap(), auction_addr);
}

#[test]
fn settle_with_auction_contract_configured_reduces_debt() {
    let (env, contract_id, borrower) = setup_defaulted_line(1_000);
    let client = CreditClient::new(&env, &contract_id);

    // A hook auction that answers the version handshake and returns the amount
    // the caller is settling for. (`MockAuction`, not the in-repo
    // `gateway-auction`: see `gateway_auction_hook_fails_closed…` below.)
    let auction_addr = wire_mock_auction(&env, &contract_id, ok_config(400));
    assert_eq!(client.get_auction_contract().unwrap(), auction_addr);

    let settlement_id = Symbol::new(&env, "auc_cfg1");

    // Settle partial — atomically invokes the configured auction hook.
    client.settle_default_liquidation(&borrower, &400_i128, &settlement_id, &10_000_u32, &None);

    assert_eq!(count_event_topic(&env, "liq_setl"), 1);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Defaulted);
    assert_eq!(line.utilized_amount, 600);
}

#[test]
fn settle_full_with_auction_contract_closes_line() {
    let (env, contract_id, borrower) = setup_defaulted_line(800);
    let client = CreditClient::new(&env, &contract_id);

    wire_mock_auction(&env, &contract_id, ok_config(800));

    // Full settlement: recovered == utilized → should close line atomically
    let settlement_id = Symbol::new(&env, "auc_full");
    client.settle_default_liquidation(&borrower, &800_i128, &settlement_id, &10_000_u32, &None);

    assert_eq!(count_event_topic(&env, "liq_setl"), 1);
    assert_eq!(count_event_topic(&env, "closed"), 1);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Closed);
    assert_eq!(line.utilized_amount, 0);
}

/// The in-repo `gateway-auction` contract now exports `get_version`, so it can
/// complete the settlement handshake. Wiring it as the hook must settle the
/// credit line through the credit-controlled CPI exactly once.
#[test]
fn gateway_auction_hook_completes_handshake_and_settles() {
    let (env, contract_id, borrower) = setup_defaulted_line(1_000);
    let client = CreditClient::new(&env, &contract_id);

    let auction_addr = env.register(Auction, ());
    client.set_auction_contract(&auction_addr);

    let settlement_id = Symbol::new(&env, "auc_real");
    setup_auction(&env, &contract_id, &auction_addr, &settlement_id, 400_i128);

    client.settle_default_liquidation(&borrower, &400_i128, &settlement_id, &10_000_u32, &None);

    assert_eq!(count_event_topic(&env, "liq_setl"), 1);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Defaulted);
    assert_eq!(line.utilized_amount, 600);
}

/// An auction that exports `settle_default_liquidation` but **not**
/// `get_version` cannot complete the settlement handshake. The credit contract
/// must fail closed — reject the settlement atomically — rather than fall back
/// to trusting an auction it could not version-check (Issue #1359). The in-repo
/// `gateway-auction` implements the handshake, so this test pins the guarantee
/// against a legacy auction that predates it.
mod legacy_auction {
    use super::*;

    #[contract]
    pub struct LegacyAuction;

    #[contractimpl]
    impl LegacyAuction {
        /// Legacy settlement entrypoint with no `get_version` companion.
        pub fn settle_default_liquidation(
            _env: Env,
            _auction_id: Symbol,
            _credit_contract: Address,
            _borrower: Address,
        ) -> i128 {
            400
        }
    }
}

#[test]
fn auction_hook_fails_closed_without_the_version_handshake() {
    let (env, contract_id, borrower) = setup_defaulted_line(1_000);
    let client = CreditClient::new(&env, &contract_id);

    let auction_addr = env.register(legacy_auction::LegacyAuction, ());
    client.set_auction_contract(&auction_addr);

    // The legacy auction would return a matching 400, but it cannot answer
    // `get_version`, so the settlement is rejected before any accounting runs.
    let settlement_id = Symbol::new(&env, "auc_legacy");
    assert_rejected_atomically(
        &env,
        &client,
        &contract_id,
        &borrower,
        &settlement_id,
        400,
        ContractError::AuctionCallFailed,
        "auction without a version handshake",
    );
}

// ── Issue #1284: zero-bid auctions settle without reverting ─────────────────

/// Without a configured auction there is no CPI that could report a zero
/// recovery, so the historical `recovered_amount > 0` guard still applies.
#[test]
fn zero_recovery_without_auction_reverts_with_5() {
    let (env, contract_id, borrower) = setup_defaulted_line(1_000);
    let client = CreditClient::new(&env, &contract_id);

    let rejected = client
        .try_settle_default_liquidation(
            &borrower,
            &0_i128,
            &Symbol::new(&env, "zero_no_auc"),
            &10_000_u32,
            &None,
        )
        .err()
        .expect("zero recovery without an auction must revert")
        .expect("the revert must be a typed contract error");
    assert_eq!(rejected, ContractError::InvalidAmount.into());
}

/// A closed auction that drew no bids reports `recovered_amount == 0`. The
/// settlement must succeed, consume the replay marker, emit a no-recovery
/// event, and leave the line `Defaulted` with its debt unchanged and its
/// pending-auction counter untouched.
#[test]
fn zero_bid_auction_settles_as_no_recovery() {
    let (env, contract_id, borrower) = setup_defaulted_line(1_000);
    let client = CreditClient::new(&env, &contract_id);
    wire_mock_auction(&env, &contract_id, ok_config(0));

    let settlement_id = Symbol::new(&env, "zero_bid");

    // Events must be read before any further contract call clears the buffer.
    client.settle_default_liquidation(&borrower, &0_i128, &settlement_id, &10_000_u32, &None);
    assert_eq!(count_event_topic(&env, "liq_norec"), 1);
    assert_eq!(count_event_topic(&env, "liq_setl"), 0);
    assert_eq!(count_event_topic(&env, "closed"), 0);

    let after = settlement_invariants(&env, &client, &contract_id, &borrower, &settlement_id);
    assert_eq!(after.utilized_amount, 1_000, "debt must be unchanged");
    assert_eq!(after.status, CreditStatus::Defaulted);
    assert!(
        after.replay_marker_written,
        "a zero-recovery settlement must consume the replay marker"
    );
    assert_eq!(
        after.pending_auctions, 1,
        "no status change must not retire the pending auction"
    );

    // The settlement id is now used: a replay is rejected.
    let replay = client
        .try_settle_default_liquidation(&borrower, &0, &settlement_id, &10_000, &None)
        .err()
        .expect("replay must revert")
        .expect("replay must surface a typed error");
    assert_eq!(replay, ContractError::AlreadyInitialized.into());

    // The admin can then write the debt off and close the line.
    client.forgive_debt(&borrower, &1_000_i128);
    client.close_credit_line(&borrower, &borrower);
    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Closed);
    assert_eq!(line.utilized_amount, 0);
    assert_eq!(client.get_pending_auction_count(), 0);
}

#[test]
fn settle_clears_reentrancy_guard_on_success() {
    let (env, contract_id, borrower) = setup_defaulted_line(500);
    let client = CreditClient::new(&env, &contract_id);

    // First settlement — should set and clear reentrancy guard
    client.settle_default_liquidation(
        &borrower,
        &200_i128,
        &Symbol::new(&env, "auc_re1"),
        &10_000_u32,
        &None,
    );

    // Second settlement with different id — proves guard was cleared
    client.settle_default_liquidation(
        &borrower,
        &100_i128,
        &Symbol::new(&env, "auc_re2"),
        &10_000_u32,
        &None,
    );

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 200); // 500 - 200 - 100
}

// ═══════════════════════════════════════════════════════════════════════════
// Recovered-amount mismatch and auction-CPI failure paths (Issue #1359)
//
// The real `gateway-auction` contract can only produce a matching recovered
// amount, so the rejection paths need an auction whose behaviour is set per
// test. `MockAuction` below exports exactly the two functions the credit
// contract calls through `AuctionClient` (`get_version` and
// `settle_default_liquidation`) and reads its behaviour from instance storage.
// ═══════════════════════════════════════════════════════════════════════════

/// Per-test behaviour of [`MockAuction`].
#[contracttype]
#[derive(Clone)]
pub struct MockAuctionConfig {
    /// `get_version` traps instead of answering (undeployed / broken auction).
    pub panic_on_get_version: bool,
    /// `settle_default_liquidation` traps after the handshake succeeded.
    pub panic_on_settle: bool,
    /// Major version reported by `get_version`; the credit contract accepts 1.
    pub version_major: u32,
    /// Amount returned by `settle_default_liquidation`.
    pub return_amount: i128,
}

#[contract]
pub struct MockAuction;

#[contractimpl]
impl MockAuction {
    /// Install a new behaviour config. Called by the tests, never by the
    /// credit contract.
    pub fn configure(env: Env, config: MockAuctionConfig) {
        env.storage().instance().set(&symbol_short!("cfg"), &config);
    }

    /// Handshake response.
    pub fn get_version(env: Env) -> creditra_credit::handshake::ProtocolVersion {
        let config = load_mock_config(&env);
        if config.panic_on_get_version {
            panic!("mock auction: get_version deliberately panicked");
        }

        creditra_credit::handshake::ProtocolVersion {
            major: config.version_major,
            minor: 0,
        }
    }

    /// Settlement response: whatever the config says, or a trap.
    pub fn settle_default_liquidation(
        env: Env,
        _auction_id: Symbol,
        _credit_contract: Address,
        _borrower: Address,
    ) -> i128 {
        let config = load_mock_config(&env);
        if config.panic_on_settle {
            panic!("mock auction: settle_default_liquidation deliberately panicked");
        }

        config.return_amount
    }
}

fn load_mock_config(env: &Env) -> MockAuctionConfig {
    env.storage()
        .instance()
        .get(&symbol_short!("cfg"))
        .unwrap_or(MockAuctionConfig {
            panic_on_get_version: false,
            panic_on_settle: false,
            version_major: 1,
            return_amount: 0,
        })
}

// ── Mock wiring ─────────────────────────────────────────────────────────────

/// A mock auction that answers the handshake and returns `amount`.
fn ok_config(amount: i128) -> MockAuctionConfig {
    MockAuctionConfig {
        panic_on_get_version: false,
        panic_on_settle: false,
        version_major: 1,
        return_amount: amount,
    }
}

/// Register a mock auction, configure it, and wire it as the credit
/// contract's settlement hook. Returns the auction address so a test can
/// reconfigure it mid-flight ("the auction catches up").
fn wire_mock_auction(env: &Env, contract_id: &Address, config: MockAuctionConfig) -> Address {
    let auction_id = env.register(MockAuction, ());
    MockAuctionClient::new(env, &auction_id).configure(&config);
    CreditClient::new(env, contract_id).set_auction_contract(&auction_id);
    auction_id
}

// ── Atomicity probes ────────────────────────────────────────────────────────

/// Every storage-visible effect a settlement rejection must leave untouched.
///
/// Events are deliberately *not* part of this snapshot: the test environment
/// keeps only the events of the most recent top-level invocation, so an event
/// count is only meaningful when read before the next contract call. The
/// rejection helper asserts event silence immediately after the rejected call
/// instead.
#[derive(Debug, PartialEq)]
struct SettlementInvariants {
    utilized_amount: i128,
    status: CreditStatus,
    /// `(symbol_short!("liq_seen"), borrower, settlement_id)` replay marker.
    replay_marker_written: bool,
    /// Lines with an in-flight liquidation auction (Issue #1169 counter).
    pending_auctions: u32,
}

/// The replay marker is written by the *accounting* half of a successful
/// settlement, as `(symbol_short!("liq_seen"), borrower, settlement_id)` in
/// persistent storage (`liquidation_settlement_key` in the credit contract's
/// lifecycle module). Reading it directly is what distinguishes "the call
/// returned an error" from "the call left no trace": a rejection that wrote
/// the marker would make every later retry of the same settlement id fail
/// with `AlreadyInitialized`.
fn replay_marker_written(
    env: &Env,
    contract_id: &Address,
    borrower: &Address,
    settlement_id: &Symbol,
) -> bool {
    let key = (
        symbol_short!("liq_seen"),
        borrower.clone(),
        settlement_id.clone(),
    );

    env.as_contract(contract_id, || env.storage().persistent().has(&key))
}

fn settlement_invariants(
    env: &Env,
    client: &CreditClient,
    contract_id: &Address,
    borrower: &Address,
    settlement_id: &Symbol,
) -> SettlementInvariants {
    let line = client.get_credit_line(borrower).unwrap();

    SettlementInvariants {
        utilized_amount: line.utilized_amount,
        status: line.status,
        replay_marker_written: replay_marker_written(env, contract_id, borrower, settlement_id),
        pending_auctions: client.get_pending_auction_count(),
    }
}

/// Reject the settlement, and prove it left nothing behind.
///
/// Asserts the exact error discriminant, then re-snapshots every observable
/// effect and requires it to be byte-identical to the pre-call snapshot.
#[track_caller]
fn assert_rejected_atomically(
    env: &Env,
    client: &CreditClient,
    contract_id: &Address,
    borrower: &Address,
    settlement_id: &Symbol,
    recovered_amount: i128,
    expected_error: ContractError,
    context: &str,
) -> SettlementInvariants {
    let before = settlement_invariants(env, client, contract_id, borrower, settlement_id);

    let rejected = client.try_settle_default_liquidation(
        borrower,
        &recovered_amount,
        settlement_id,
        &10_000_u32,
        &None,
    );

    // Read the event log first: the buffer still holds exactly the events of
    // the rejected call, so this asserts the rejection told indexers nothing.
    assert_eq!(
        count_event_topic(env, "liq_setl"),
        0,
        "{context}: rejected settlement emitted a liq_setl event"
    );
    assert_eq!(
        count_event_topic(env, "closed"),
        0,
        "{context}: rejected settlement emitted a closed event"
    );

    let error = rejected
        .err()
        .unwrap_or_else(|| panic!("{context}: settlement was expected to revert"))
        .unwrap_or_else(|host| panic!("{context}: expected a typed contract error, got {host:?}"));

    assert_eq!(
        error,
        expected_error.into(),
        "{context}: wrong error discriminant"
    );

    assert_eq!(
        settlement_invariants(env, client, contract_id, borrower, settlement_id),
        before,
        "{context}: rejected settlement mutated observable state"
    );

    before
}

// ── Acceptance criterion: mismatch reverts with #62, no state change ────────

#[test]
fn recovered_amount_mismatch_reverts_with_62() {
    let (env, contract_id, borrower) = setup_defaulted_line(1_000);
    let client = CreditClient::new(&env, &contract_id);
    // The auction closed at 399; the caller claims 400.
    wire_mock_auction(&env, &contract_id, ok_config(399));

    let settlement_id = Symbol::new(&env, "mismatch");
    let before = assert_rejected_atomically(
        &env,
        &client,
        &contract_id,
        &borrower,
        &settlement_id,
        400,
        ContractError::AuctionCallFailed,
        "recovered-amount mismatch",
    );

    // Preconditions: the line was settleable and _not_ already marked.
    assert_eq!(before.utilized_amount, 1_000);
    assert_eq!(before.status, CreditStatus::Defaulted);
    assert!(
        !before.replay_marker_written,
        "mismatch must not consume the replay marker"
    );
    assert_eq!(
        before.pending_auctions, 1,
        "mismatch must not retire the pending auction"
    );
}

#[test]
fn auction_over_return_reverts_with_62() {
    // Returning *more* than the caller claimed is still a mismatch: the
    // settlement accounting must agree with the auction exactly, in both
    // directions, or a transposed amount silently under-credits the line.
    let (env, contract_id, borrower) = setup_defaulted_line(1_000);
    let client = CreditClient::new(&env, &contract_id);
    wire_mock_auction(&env, &contract_id, ok_config(401));

    assert_rejected_atomically(
        &env,
        &client,
        &contract_id,
        &borrower,
        &Symbol::new(&env, "over"),
        400,
        ContractError::AuctionCallFailed,
        "auction over-return",
    );
}

// ── Acceptance criterion: version CPI failure reverts with #62 ──────────────

#[test]
fn get_version_cpi_failure_reverts_with_62() {
    let (env, contract_id, borrower) = setup_defaulted_line(1_000);
    let client = CreditClient::new(&env, &contract_id);
    wire_mock_auction(
        &env,
        &contract_id,
        MockAuctionConfig {
            panic_on_get_version: true,
            ..ok_config(400)
        },
    );

    let settlement_id = Symbol::new(&env, "ver_cpi");
    let before = assert_rejected_atomically(
        &env,
        &client,
        &contract_id,
        &borrower,
        &settlement_id,
        400,
        ContractError::AuctionCallFailed,
        "get_version CPI failure",
    );

    assert!(!before.replay_marker_written);
    assert_eq!(before.pending_auctions, 1);
}

// ── Acceptance criterion: incompatible major reverts with #61 ───────────────

#[test]
fn incompatible_major_version_reverts_with_61() {
    let (env, contract_id, borrower) = setup_defaulted_line(1_000);
    let client = CreditClient::new(&env, &contract_id);
    wire_mock_auction(
        &env,
        &contract_id,
        MockAuctionConfig {
            version_major: 2,
            ..ok_config(400)
        },
    );

    let settlement_id = Symbol::new(&env, "ver_major");
    let before = assert_rejected_atomically(
        &env,
        &client,
        &contract_id,
        &borrower,
        &settlement_id,
        400,
        ContractError::IncompatibleVersion,
        "incompatible major version",
    );

    // A version mismatch is rejected before the settlement CPI, so the
    // accounting half is never reached and nothing can have been written.
    assert!(!before.replay_marker_written);
    assert_eq!(before.pending_auctions, 1);
}

// ── Settlement CPI failure (the third rejection path in the hook) ───────────

#[test]
fn settle_cpi_failure_reverts_with_62() {
    let (env, contract_id, borrower) = setup_defaulted_line(1_000);
    let client = CreditClient::new(&env, &contract_id);
    wire_mock_auction(
        &env,
        &contract_id,
        MockAuctionConfig {
            panic_on_settle: true,
            ..ok_config(400)
        },
    );

    assert_rejected_atomically(
        &env,
        &client,
        &contract_id,
        &borrower,
        &Symbol::new(&env, "stl_cpi"),
        400,
        ContractError::AuctionCallFailed,
        "settle CPI failure",
    );
}

// ── Acceptance criterion: a correct retry succeeds ─────────────────────────

#[test]
fn correct_retry_after_mismatch_reuses_the_same_settlement_id() {
    let (env, contract_id, borrower) = setup_defaulted_line(1_000);
    let client = CreditClient::new(&env, &contract_id);
    let auction_id = wire_mock_auction(&env, &contract_id, ok_config(399));
    let settlement_id = Symbol::new(&env, "retry_mm");

    assert_rejected_atomically(
        &env,
        &client,
        &contract_id,
        &borrower,
        &settlement_id,
        400,
        ContractError::AuctionCallFailed,
        "recovered-amount mismatch",
    );

    // The auction state catches up with the credit contract.
    MockAuctionClient::new(&env, &auction_id).configure(&ok_config(400));

    // Retry the *same* settlement id. This is the regression the issue asks
    // for: if the rejected attempt had consumed the replay marker, this call
    // would revert with `AlreadyInitialized` and the line would be stuck in
    // `Defaulted` with no way to settle it.
    client
        .try_settle_default_liquidation(&borrower, &400, &settlement_id, &10_000, &None)
        .expect("retry after a mismatch must not be blocked by replay protection")
        .expect("retry must not surface a contract error");

    // Events first: any further contract call would clear the event buffer.
    assert_eq!(count_event_topic(&env, "liq_setl"), 1);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 600, "retry must apply the recovery");
    assert_eq!(line.status, CreditStatus::Defaulted);
    assert!(
        replay_marker_written(&env, &contract_id, &borrower, &settlement_id),
        "a successful settlement must consume the replay marker"
    );

    // Only now is a replay of the same id rejected: the marker belongs to the
    // successful attempt, not to the failed one.
    let replay = client
        .try_settle_default_liquidation(&borrower, &400, &settlement_id, &10_000, &None)
        .err()
        .expect("replay after a successful settlement must revert")
        .expect("replay must surface a typed contract error");
    assert_eq!(replay, ContractError::AlreadyInitialized.into());
}

#[test]
fn correct_retry_after_version_failure_closes_the_line() {
    let (env, contract_id, borrower) = setup_defaulted_line(400);
    let client = CreditClient::new(&env, &contract_id);
    let auction_id = wire_mock_auction(
        &env,
        &contract_id,
        MockAuctionConfig {
            version_major: 99,
            ..ok_config(400)
        },
    );
    let settlement_id = Symbol::new(&env, "retry_ver");

    assert_rejected_atomically(
        &env,
        &client,
        &contract_id,
        &borrower,
        &settlement_id,
        400,
        ContractError::IncompatibleVersion,
        "incompatible major version",
    );

    // "Upgrade" the auction and retry the same id: a full recovery must close
    // the line and retire the pending auction.
    MockAuctionClient::new(&env, &auction_id).configure(&ok_config(400));
    client
        .try_settle_default_liquidation(&borrower, &400, &settlement_id, &10_000, &None)
        .expect("retry after a version handshake failure must be allowed")
        .expect("retry must not surface a contract error");

    // Events first: any further contract call would clear the event buffer.
    assert_eq!(count_event_topic(&env, "liq_setl"), 1);
    assert_eq!(count_event_topic(&env, "closed"), 1);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 0);
    assert_eq!(line.status, CreditStatus::Closed);
    assert_eq!(
        client.get_pending_auction_count(),
        0,
        "a full settlement retires the pending auction"
    );
}

/// Every rejection path, end to end: reject → retry the same settlement id
/// after fixing the auction → settle. Failing any leg means a single auction
/// glitch permanently strands a defaulted line.
#[test]
fn every_rejection_path_leaves_the_line_re_settleable() {
    type FailureMode = (&'static str, fn(i128) -> MockAuctionConfig, ContractError);

    let failure_modes: [FailureMode; 5] = [
        (
            "auction returns less than claimed",
            |claimed| ok_config(claimed - 1),
            ContractError::AuctionCallFailed,
        ),
        (
            "auction returns more than claimed",
            |claimed| ok_config(claimed + 1),
            ContractError::AuctionCallFailed,
        ),
        (
            "get_version CPI failure",
            |claimed| MockAuctionConfig {
                panic_on_get_version: true,
                ..ok_config(claimed)
            },
            ContractError::AuctionCallFailed,
        ),
        (
            "incompatible major version",
            |claimed| MockAuctionConfig {
                version_major: 2,
                ..ok_config(claimed)
            },
            ContractError::IncompatibleVersion,
        ),
        (
            "settle CPI failure",
            |claimed| MockAuctionConfig {
                panic_on_settle: true,
                ..ok_config(claimed)
            },
            ContractError::AuctionCallFailed,
        ),
    ];

    for (label, broken_config, expected_error) in failure_modes {
        // A fresh environment per mode so the counters and events start clean.
        let (env, contract_id, borrower) = setup_defaulted_line(1_000);
        let client = CreditClient::new(&env, &contract_id);
        let auction_id = wire_mock_auction(&env, &contract_id, broken_config(400));
        let settlement_id = Symbol::new(&env, "same_id");

        assert_rejected_atomically(
            &env,
            &client,
            &contract_id,
            &borrower,
            &settlement_id,
            400,
            expected_error,
            label,
        );

        MockAuctionClient::new(&env, &auction_id).configure(&ok_config(400));
        client
            .try_settle_default_liquidation(&borrower, &400, &settlement_id, &10_000, &None)
            .unwrap_or_else(|host| panic!("{label}: retry was rejected: {host:?}"))
            .unwrap_or_else(|error| panic!("{label}: retry returned {error:?}"));

        // Events first: any further contract call would clear the buffer.
        assert_eq!(count_event_topic(&env, "liq_setl"), 1, "{label}");
        assert_eq!(
            client.get_credit_line(&borrower).unwrap().utilized_amount,
            600,
            "{label}: retry must apply the recovery"
        );
    }
}
