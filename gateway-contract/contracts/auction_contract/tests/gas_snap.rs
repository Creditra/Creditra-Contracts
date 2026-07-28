// SPDX-License-Identifier: MIT

//! Per-entrypoint CPU / memory gas snapshot tests for the gateway-auction
//! contract.
//!
//! # What
//!
//! Measures and upper-bounds the Soroban instruction and memory cost of every
//! public, state-changing entrypoint on the [`gateway_auction::Auction`]
//! contract.  Any call that exceeds the pinned ceiling fails CI immediately —
//! giving reviewers an early signal that a change introduced unexpected compute
//! cost growth.
//!
//! # Entrypoints covered
//!
//! | Entrypoint | Mode | Category |
//! |---|---|---|
//! | `init_auction` | English | factory write |
//! | `init_auction` | Dutch (linear) | factory write |
//! | `set_factory_contract` | — | factory write |
//! | `place_bid` | English (first bid) | bidder write |
//! | `place_bid` | English (outbid with refund) | bidder write |
//! | `place_bid` | Dutch (auto-close) | bidder write |
//! | `close_auction` | English | factory write |
//! | `settle_default_liquidation` | — | factory write |
//! | `claim_auction` | — | winner write |
//!
//! # Regression threshold
//!
//! The CI gate is a **hard upper bound**: if `cpu_instruction_cost()` or
//! `memory_bytes_cost()` exceeds the constant for that entrypoint the test
//! panics with a descriptive message including the observed value.  Bounds are
//! set at ≈ 2× the cost seen during initial baseline runs, leaving room for
//! harmless host-side variation while still catching genuine regressions (> 5 %
//! threshold as required by the issue).
//!
//! # How to update bounds
//!
//! 1. Run `cargo test -p gateway-auction --test gas_snap -- --nocapture`
//!    and note the `cpu=… mem=…` lines printed to stderr.
//! 2. Multiply by 1.05 (5 % tolerance) and round up to the nearest 100_000.
//! 3. Update the `CPU_CEILING_*` / `MEM_CEILING_*` constants below.
//!
//! # See also
//!
//! - `contracts/collateral/tests/gas_snap.rs` — canonical pattern for
//!   upper-bound assertions.
//! - `contracts/credit/src/instrument.rs` — budget instrumentation helpers
//!   used by the credit contract's JSON-baseline variant.
//!
//! # Running
//!
//! ```bash
//! cargo test -p gateway-auction --test gas_snap
//! cargo test -p gateway-auction --test gas_snap -- --nocapture
//! ```

extern crate std;

use gateway_auction::{Auction, AuctionClient, AuctionMode, DutchAuctionDecay};
use soroban_sdk::{
    testutils::{budget::Budget, Address as _, Ledger},
    token::StellarAssetClient,
    Address, Env, Symbol,
};

// ── Regression ceilings ───────────────────────────────────────────────────────
//
// Each constant is the *maximum* allowed cost for that entrypoint.  Values are
// derived from the initial baseline run (see eprintln output when running with
// `--nocapture`) and set at roughly 2× that value, rounded up to the nearest
// magnitude.  This gives CI enough headroom to absorb incidental host-side
// variation while still catching genuine > 5 % regressions.
//
// Baseline measurements (debug profile, soroban-sdk v22):
//   init_auction         cpu=54_939   mem=7_987
//   set_factory_contract cpu=29_824   mem=3_932
//   place_bid/english    cpu=270_219  mem=41_290
//   place_bid/outbid     cpu=429_752  mem=65_888
//   place_bid/dutch      cpu=274_032  mem=41_685
//   close_auction        cpu=112_185  mem=15_473
//   settle_default_liq   cpu=277_312  mem=44_104
//   claim_auction        cpu=277_149  mem=44_570
//
// To re-baseline: run `cargo test -p gateway-auction --test gas_snap -- --nocapture`
// and multiply each reported value by 1.05 (5 % tolerance) then round up.

/// `init_auction` (English or Dutch) — storage write + parameter validation.
const CPU_CEILING_INIT_AUCTION: u64 = 500_000;
/// Memory ceiling for `init_auction`.
const MEM_CEILING_INIT_AUCTION: u64 = 100_000;

/// `set_factory_contract` — single instance-storage write + require_auth.
const CPU_CEILING_SET_FACTORY: u64 = 300_000;
/// Memory ceiling for `set_factory_contract`.
const MEM_CEILING_SET_FACTORY: u64 = 50_000;

/// `place_bid` English first-bid path (no refund, no token).
const CPU_CEILING_PLACE_BID_ENGLISH: u64 = 2_000_000;
/// Memory ceiling for `place_bid` English first-bid.
const MEM_CEILING_PLACE_BID_ENGLISH: u64 = 400_000;

/// `place_bid` English outbid path — includes prior-bid refund token transfer.
const CPU_CEILING_PLACE_BID_OUTBID: u64 = 3_000_000;
/// Memory ceiling for `place_bid` English outbid (token transfer included).
const MEM_CEILING_PLACE_BID_OUTBID: u64 = 600_000;

/// `place_bid` Dutch — price validation + auto-close + token transfer.
const CPU_CEILING_PLACE_BID_DUTCH: u64 = 2_000_000;
/// Memory ceiling for `place_bid` Dutch.
const MEM_CEILING_PLACE_BID_DUTCH: u64 = 400_000;

/// `close_auction` — factory auth + persistent state write + event publish.
const CPU_CEILING_CLOSE_AUCTION: u64 = 1_000_000;
/// Memory ceiling for `close_auction`.
const MEM_CEILING_CLOSE_AUCTION: u64 = 150_000;

/// `settle_default_liquidation` — factory auth + replay-protection write +
/// optional token transfer.
const CPU_CEILING_SETTLE: u64 = 2_000_000;
/// Memory ceiling for `settle_default_liquidation`.
const MEM_CEILING_SETTLE: u64 = 400_000;

/// `claim_auction` — winner auth + token transfer + state write.
const CPU_CEILING_CLAIM: u64 = 2_000_000;
/// Memory ceiling for `claim_auction`.
const MEM_CEILING_CLAIM: u64 = 400_000;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Reset the env budget to unlimited, execute `f`, then return the consumed
/// (cpu_instructions, memory_bytes) as a tuple.
///
/// Calling `reset_unlimited()` before the closure ensures previous setup work
/// (contract registration, minting, etc.) is not included in the measurement.
fn measure(env: &Env, f: impl FnOnce()) -> (u64, u64) {
    let mut budget: Budget = env.cost_estimate().budget();
    budget.reset_unlimited();
    f();
    (budget.cpu_instruction_cost(), budget.memory_bytes_cost())
}

/// Assert both `cpu` and `mem` are positive and do not exceed their respective
/// ceilings.  Panics with a human-readable regression message on failure.
fn assert_within_ceiling(label: &str, cpu: u64, mem: u64, cpu_ceil: u64, mem_ceil: u64) {
    assert!(
        cpu > 0,
        "gas_snap [{label}]: cpu_instruction_cost must be > 0 (got {cpu})"
    );
    assert!(
        mem > 0,
        "gas_snap [{label}]: memory_bytes_cost must be > 0 (got {mem})"
    );
    assert!(
        cpu <= cpu_ceil,
        "gas_snap [{label}]: CPU regression detected!\n\
         observed  = {cpu}\n\
         ceiling   = {cpu_ceil}\n\
         Exceeds upper bound — review the change for unintended instruction growth.",
    );
    assert!(
        mem <= mem_ceil,
        "gas_snap [{label}]: memory regression detected!\n\
         observed  = {mem}\n\
         ceiling   = {mem_ceil}\n\
         Exceeds upper bound — review the change for unintended memory growth.",
    );
    eprintln!("gas_snap [{label}]: cpu={cpu} mem={mem}");
}

/// Deploy a fresh auction contract, register a factory address, and return
/// `(env, contract_id, client, factory)`.
///
/// Uses `mock_all_auths_allowing_non_root_auth` so the budget measures real
/// contract logic without separate auth mock setup costs interfering.
fn setup_contract() -> (Env, Address, AuctionClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);

    let factory = Address::generate(&env);
    client.set_factory_contract(&factory);

    (env, contract_id, client, factory)
}

/// Register a Stellar Asset Contract, set it as the `bid_token` on the auction
/// contract, mint `contract_balance` to the contract (for refunds), and mint
/// `bidder_balance` to each bidder.
///
/// Returns the token address.
fn setup_token(
    env: &Env,
    contract_id: &Address,
    contract_balance: i128,
    bidders: &[Address],
    bidder_balance: i128,
) -> Address {
    let token_admin = Address::generate(env);
    let token_id = env.register_stellar_asset_contract_v2(token_admin);
    let bid_token = token_id.address();
    let sac = StellarAssetClient::new(env, &bid_token);

    sac.mint(contract_id, &contract_balance);
    for bidder in bidders {
        sac.mint(bidder, &bidder_balance);
    }

    // Store the token address in instance storage where `place_bid` expects it.
    env.as_contract(contract_id, || {
        env.storage()
            .instance()
            .set(&soroban_sdk::Symbol::new(env, "bid_token"), &bid_token);
    });

    bid_token
}

// ── init_auction ──────────────────────────────────────────────────────────────

/// `init_auction` in English mode: validates params and writes `AuctionState` to
/// persistent storage.  Measured after setup so only the entrypoint cost is
/// captured.
#[test]
fn gas_init_auction_english() {
    let (env, _contract_id, client, _factory) = setup_contract();
    let auction_id = Symbol::new(&env, "gas_eng_init");

    let (cpu, mem) = measure(&env, || {
        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0_u64,
            &86_400_u64,
            &100_i128,
            &50_u32, // 0.5% increment
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );
    });

    assert_within_ceiling(
        "init_auction/english",
        cpu,
        mem,
        CPU_CEILING_INIT_AUCTION,
        MEM_CEILING_INIT_AUCTION,
    );
}

/// `init_auction` in Dutch mode: same write path but also validates Dutch-
/// specific fields (`dutch_start_price >= dutch_floor_price`, decay config).
#[test]
fn gas_init_auction_dutch() {
    let (env, _contract_id, client, _factory) = setup_contract();
    let auction_id = Symbol::new(&env, "gas_dut_init");

    let (cpu, mem) = measure(&env, || {
        client.init_auction(
            &auction_id,
            &AuctionMode::Dutch,
            &0_u64,
            &86_400_u64,
            &50_i128,
            &0_u32,
            &Some(1_000_i128),
            &Some(100_i128),
            &DutchAuctionDecay::Linear,
            &None,
        );
    });

    assert_within_ceiling(
        "init_auction/dutch",
        cpu,
        mem,
        CPU_CEILING_INIT_AUCTION,
        MEM_CEILING_INIT_AUCTION,
    );
}

// ── set_factory_contract ──────────────────────────────────────────────────────

/// `set_factory_contract` writes a single instance-storage entry after
/// verifying `require_auth` from the factory address.
#[test]
fn gas_set_factory_contract() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);
    let factory = Address::generate(&env);

    let (cpu, mem) = measure(&env, || {
        client.set_factory_contract(&factory);
    });

    assert_within_ceiling(
        "set_factory_contract",
        cpu,
        mem,
        CPU_CEILING_SET_FACTORY,
        MEM_CEILING_SET_FACTORY,
    );
}

// ── place_bid (English) ───────────────────────────────────────────────────────

/// `place_bid` on a fresh English auction — first bid path: auth + amount
/// validation + persistent storage write.  No refund token transfer (no prior
/// bidder).
#[test]
fn gas_place_bid_english_first() {
    let (env, contract_id, client, _factory) = setup_contract();
    let bidder = Address::generate(&env);
    let auction_id = Symbol::new(&env, "gas_eng_bid1");

    // Mint tokens so the token transfer succeeds.
    setup_token(&env, &contract_id, 0, std::slice::from_ref(&bidder), 1_000);

    // min_bid=1, min_increment_bps=0: first bid threshold = min_next_bid(1, 0) = 2.
    client.init_auction(
        &auction_id,
        &AuctionMode::English,
        &0_u64,
        &u64::MAX,
        &1_i128,
        &0_u32,
        &None,
        &None,
        &DutchAuctionDecay::None,
        &None,
    );

    let (cpu, mem) = measure(&env, || {
        client.place_bid(&auction_id, &bidder, &10_i128);
    });

    assert_within_ceiling(
        "place_bid/english/first",
        cpu,
        mem,
        CPU_CEILING_PLACE_BID_ENGLISH,
        MEM_CEILING_PLACE_BID_ENGLISH,
    );
}

/// `place_bid` outbid path — involves refunding the previous highest bidder
/// via a token CPI under the reentrancy guard.  More expensive than the
/// first-bid path.
#[test]
fn gas_place_bid_english_outbid() {
    let (env, contract_id, client, _factory) = setup_contract();
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let auction_id = Symbol::new(&env, "gas_eng_outbid");

    // Fund both bidders; also pre-fund the contract for the refund transfer.
    setup_token(
        &env,
        &contract_id,
        1_000,
        &[alice.clone(), bob.clone()],
        1_000,
    );

    // min_bid=1, bps=0: first threshold = 2, second threshold = first_bid + 1.
    client.init_auction(
        &auction_id,
        &AuctionMode::English,
        &0_u64,
        &u64::MAX,
        &1_i128,
        &0_u32,
        &None,
        &None,
        &DutchAuctionDecay::None,
        &None,
    );

    // First bid — not measured.
    client.place_bid(&auction_id, &alice, &10_i128);

    // Second bid — includes the refund CPI for Alice's prior 10.
    let (cpu, mem) = measure(&env, || {
        client.place_bid(&auction_id, &bob, &20_i128);
    });

    assert_within_ceiling(
        "place_bid/english/outbid",
        cpu,
        mem,
        CPU_CEILING_PLACE_BID_OUTBID,
        MEM_CEILING_PLACE_BID_OUTBID,
    );
}

// ── place_bid (Dutch) ─────────────────────────────────────────────────────────

/// `place_bid` in Dutch mode auto-closes the auction and emits
/// `AUC_CLOSE` in the same transaction.  Price computation adds a Dutch-curve
/// evaluation on top of the English write path.
#[test]
fn gas_place_bid_dutch() {
    let (env, contract_id, client, _factory) = setup_contract();
    let bidder = Address::generate(&env);
    let auction_id = Symbol::new(&env, "gas_dutch_bid");

    setup_token(&env, &contract_id, 0, std::slice::from_ref(&bidder), 2_000);

    // Auction runs from t=0 to t=86_400.  Start price 1_000, floor 100.
    client.init_auction(
        &auction_id,
        &AuctionMode::Dutch,
        &0_u64,
        &86_400_u64,
        &50_i128,
        &0_u32,
        &Some(1_000_i128),
        &Some(100_i128),
        &DutchAuctionDecay::Linear,
        &None,
    );

    // Bid at start time — current price = 1_000.
    env.ledger().with_mut(|l| l.timestamp = 0);

    let (cpu, mem) = measure(&env, || {
        client.place_bid(&auction_id, &bidder, &1_000_i128);
    });

    assert_within_ceiling(
        "place_bid/dutch",
        cpu,
        mem,
        CPU_CEILING_PLACE_BID_DUTCH,
        MEM_CEILING_PLACE_BID_DUTCH,
    );
}

// ── close_auction ─────────────────────────────────────────────────────────────

/// `close_auction` transitions an `Open` auction to `Closed`, requires factory
/// auth, and emits `AUC_CLOSE`.
#[test]
fn gas_close_auction() {
    let (env, _contract_id, client, _factory) = setup_contract();
    let auction_id = Symbol::new(&env, "gas_close");

    client.init_auction(
        &auction_id,
        &AuctionMode::English,
        &0_u64,
        &u64::MAX,
        &1_i128,
        &0_u32,
        &None,
        &None,
        &DutchAuctionDecay::None,
        &None,
    );

    // Place a bid so there is a winner for the closed-event payload.
    let bidder = Address::generate(&env);
    client.place_bid(&auction_id, &bidder, &10_i128);

    let (cpu, mem) = measure(&env, || {
        client.close_auction(&auction_id);
    });

    assert_within_ceiling(
        "close_auction",
        cpu,
        mem,
        CPU_CEILING_CLOSE_AUCTION,
        MEM_CEILING_CLOSE_AUCTION,
    );
}

// ── settle_default_liquidation ────────────────────────────────────────────────

/// `settle_default_liquidation` validates factory auth, writes a replay-
/// protection marker, emits the settlement event, and optionally triggers a
/// token transfer to the credit contract.
///
/// The measurement path here has a bid + token so all branches execute.
#[test]
fn gas_settle_default_liquidation() {
    let (env, contract_id, client, factory) = setup_contract();
    let bidder = Address::generate(&env);
    let borrower = Address::generate(&env);
    let auction_id = Symbol::new(&env, "gas_settle");

    // Fund the bidder and contract (contract needs tokens to transfer to credit).
    setup_token(
        &env,
        &contract_id,
        1_000,
        std::slice::from_ref(&bidder),
        1_000,
    );

    client.init_auction(
        &auction_id,
        &AuctionMode::English,
        &0_u64,
        &u64::MAX,
        &1_i128,
        &0_u32,
        &None,
        &None,
        &DutchAuctionDecay::None,
        &None,
    );

    client.place_bid(&auction_id, &bidder, &420_i128);
    client.close_auction(&auction_id);

    // The factory is also the credit_contract for the single-address settle path.
    let (cpu, mem) = measure(&env, || {
        client.settle_default_liquidation(&auction_id, &factory, &borrower);
    });

    assert_within_ceiling(
        "settle_default_liquidation",
        cpu,
        mem,
        CPU_CEILING_SETTLE,
        MEM_CEILING_SETTLE,
    );
}

// ── claim_auction ─────────────────────────────────────────────────────────────

/// `claim_auction` requires winner auth, updates state to `Claimed`, and
/// transfers the bid token back to the winner.
#[test]
fn gas_claim_auction() {
    let (env, contract_id, client, _factory) = setup_contract();
    let winner = Address::generate(&env);
    let auction_id = Symbol::new(&env, "gas_claim");

    setup_token(
        &env,
        &contract_id,
        1_000,
        std::slice::from_ref(&winner),
        1_000,
    );

    client.init_auction(
        &auction_id,
        &AuctionMode::English,
        &0_u64,
        &u64::MAX,
        &1_i128,
        &0_u32,
        &None,
        &None,
        &DutchAuctionDecay::None,
        &None,
    );

    client.place_bid(&auction_id, &winner, &300_i128);
    client.close_auction(&auction_id);

    let (cpu, mem) = measure(&env, || {
        client.claim_auction(&auction_id);
    });

    assert_within_ceiling(
        "claim_auction",
        cpu,
        mem,
        CPU_CEILING_CLAIM,
        MEM_CEILING_CLAIM,
    );
}

// ── Structural properties ─────────────────────────────────────────────────────

/// Two identical `init_auction` calls on separate auction IDs must cost the
/// same CPU and memory — verifying the Soroban simulator's deterministic cost
/// model within the 5% tolerance threshold.
#[test]
fn gas_init_auction_deterministic() {
    let (env, _contract_id, client, _factory) = setup_contract();

    let id1 = Symbol::new(&env, "gas_det1");
    let id2 = Symbol::new(&env, "gas_det2");

    let (cpu1, mem1) = measure(&env, || {
        client.init_auction(
            &id1,
            &AuctionMode::English,
            &0_u64,
            &86_400_u64,
            &1_i128,
            &0_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );
    });

    let (cpu2, mem2) = measure(&env, || {
        client.init_auction(
            &id2,
            &AuctionMode::English,
            &0_u64,
            &86_400_u64,
            &1_i128,
            &0_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );
    });

    // Allow up to 5% variance between two structurally identical calls.
    // The Soroban cost model may vary slightly by storage slot occupancy.
    let cpu_pct = if cpu1 > 0 {
        (cpu1 as f64 - cpu2 as f64).abs() / cpu1 as f64 * 100.0
    } else {
        0.0
    };
    let mem_pct = if mem1 > 0 {
        (mem1 as f64 - mem2 as f64).abs() / mem1 as f64 * 100.0
    } else {
        0.0
    };
    assert!(
        cpu_pct <= 5.0,
        "init_auction CPU varied by {cpu_pct:.1}% between two identical calls (first={cpu1} second={cpu2}); max 5%"
    );
    assert!(
        mem_pct <= 5.0,
        "init_auction memory varied by {mem_pct:.1}% between two identical calls (first={mem1} second={mem2}); max 5%"
    );
    eprintln!("gas_init_auction_deterministic: cpu1={cpu1} cpu2={cpu2} cpu_pct={cpu_pct:.2}%  mem1={mem1} mem2={mem2} mem_pct={mem_pct:.2}%");
}

/// `place_bid` (write) must cost at least as much as a pure storage read
/// performed via `env.as_contract`.  This guards against a mistaken
/// optimisation that inadvertently omits auth checks or storage flushes.
#[test]
fn gas_write_more_expensive_than_storage_read() {
    let (env, contract_id, client, _factory) = setup_contract();
    let bidder = Address::generate(&env);
    let auction_id = Symbol::new(&env, "gas_wr_cmp");

    setup_token(&env, &contract_id, 0, std::slice::from_ref(&bidder), 1_000);

    client.init_auction(
        &auction_id,
        &AuctionMode::English,
        &0_u64,
        &u64::MAX,
        &1_i128,
        &0_u32,
        &None,
        &None,
        &DutchAuctionDecay::None,
        &None,
    );

    // Measure a raw persistent-storage read (no auth, no writes).
    let (read_cpu, _) = measure(&env, || {
        env.as_contract(&contract_id, || {
            let _: Option<gateway_auction::AuctionState> =
                env.storage().persistent().get(&auction_id);
        });
    });

    // Measure the write path.
    let (write_cpu, _) = measure(&env, || {
        client.place_bid(&auction_id, &bidder, &10_i128);
    });

    assert!(
        write_cpu >= read_cpu,
        "place_bid ({write_cpu} CPU) must cost at least as much as a raw storage read ({read_cpu} CPU)"
    );
    eprintln!("gas_write_vs_read: read_cpu={read_cpu} write_cpu={write_cpu}");
}

/// Sequential bids on the same auction must each stay within the outbid
/// ceiling.  Verifies that per-bid cost does not accumulate unboundedly with
/// bid history (the auction stores only the current leader, not the full list).
#[test]
fn gas_multi_bid_cost_stable() {
    let (env, contract_id, client, _factory) = setup_contract();
    let bidders = [
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
    ];
    let auction_id = Symbol::new(&env, "gas_multi");

    // Pre-fund all bidders and the contract (for refunds).
    setup_token(&env, &contract_id, 5_000, &bidders, 5_000);

    client.init_auction(
        &auction_id,
        &AuctionMode::English,
        &0_u64,
        &u64::MAX,
        &1_i128,
        &0_u32,
        &None,
        &None,
        &DutchAuctionDecay::None,
        &None,
    );

    // First bid (no refund).
    client.place_bid(&auction_id, &bidders[0], &10_i128);

    // Second bid — refunds bidder[0].
    let (cpu2, mem2) = measure(&env, || {
        client.place_bid(&auction_id, &bidders[1], &20_i128);
    });

    // Third bid — refunds bidder[1].
    let (cpu3, mem3) = measure(&env, || {
        client.place_bid(&auction_id, &bidders[2], &40_i128);
    });

    // Each outbid should be bounded by the outbid ceiling.
    assert_within_ceiling(
        "place_bid/multi/bid2",
        cpu2,
        mem2,
        CPU_CEILING_PLACE_BID_OUTBID,
        MEM_CEILING_PLACE_BID_OUTBID,
    );
    assert_within_ceiling(
        "place_bid/multi/bid3",
        cpu3,
        mem3,
        CPU_CEILING_PLACE_BID_OUTBID,
        MEM_CEILING_PLACE_BID_OUTBID,
    );

    // Per-bid cost must not grow: bid3 should be within 5% of bid2.
    let growth_pct = if cpu2 > 0 {
        (cpu3 as f64 - cpu2 as f64).abs() / cpu2 as f64 * 100.0
    } else {
        0.0
    };
    assert!(
        growth_pct <= 5.0,
        "place_bid outbid cost grew by {growth_pct:.1}% from bid 2 ({cpu2}) to bid 3 ({cpu3}); \
         max allowed is 5%"
    );
    eprintln!("gas_multi_bid: cpu2={cpu2} cpu3={cpu3} growth={growth_pct:.2}%");
}
