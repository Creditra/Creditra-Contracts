//! Exhaustive `AuctionStatus` transition matrix (Issue #614).
//!
//! # State machine
//!
//! ```text
//!   Open ──close_auction / Dutch place_bid──► Closed ──claim_auction──► Claimed
//!     │                                          │
//!     └── place_bid (English) stays Open       └── terminal after claim
//! ```
//!
//! # Cross-product (starting status × entrypoint)
//!
//! | From    | `place_bid`              | `close_auction`      | `claim_auction`        |
//! |---------|--------------------------|----------------------|------------------------|
//! | Open    | ✓ (English: Open)        | ✓ → Closed           | ✗ `NotClosed=9`        |
//! |         | ✓ (Dutch: → Closed)      |                      |                        |
//! | Closed  | ✗ `NotOpen=8`            | ✗ `NotOpen=8`        | ✓ → Claimed            |
//! | Claimed | ✗ `NotOpen=8`            | ✗ `AlreadyClaimed=2` | ✗ `NotClosed=9`        |
//!
//! Six illegal pairs per mode; three legal pairs per mode. Every illegal pair must
//! revert with the documented `AuctionError` discriminant and leave stored status
//! unchanged.
//!
//! # Running
//!
//! ```bash
//! cargo test -p gateway-auction --test transition_matrix
//! ```

use soroban_sdk::testutils::Ledger as _;
use gateway_auction::{
    Auction, AuctionClient, AuctionError, AuctionMode, AuctionState, AuctionStatus,
    DutchAuctionDecay,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Symbol};
/// Entrypoints that can attempt an `AuctionStatus` transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Entrypoint {
    PlaceBid,
    CloseAuction,
    ClaimAuction,
}

/// One row of the status × entrypoint matrix for a given auction mode.
#[derive(Clone, Debug)]
struct TransitionCase {
    label: &'static str,
    mode: AuctionMode,
    from: AuctionStatus,
    entrypoint: Entrypoint,
    expect_ok: bool,
    expected_error: Option<AuctionError>,
    expected_to: Option<AuctionStatus>,
    /// Minimum qualifying bid for `PlaceBid` when `expect_ok` is true.
    qualifying_bid: i128,
}

fn transition_matrix(mode: AuctionMode) -> Vec<TransitionCase> {
    let open_place_bid_to = match mode {
        AuctionMode::English => AuctionStatus::Open,
        AuctionMode::Dutch => AuctionStatus::Closed,
    };
    let open_place_bid_bid = match mode {
        AuctionMode::English => 200_i128,
        AuctionMode::Dutch => 500_i128,
    };
    vec![
        TransitionCase {
            label: "Open + place_bid",
            mode,
            from: AuctionStatus::Open,
            entrypoint: Entrypoint::PlaceBid,
            expect_ok: true,
            expected_error: None,
            expected_to: Some(open_place_bid_to),
            qualifying_bid: open_place_bid_bid,
        },
        TransitionCase {
            label: "Open + close_auction → Closed",
            mode,
            from: AuctionStatus::Open,
            entrypoint: Entrypoint::CloseAuction,
            expect_ok: true,
            expected_error: None,
            expected_to: Some(AuctionStatus::Closed),
            qualifying_bid: 0,
        },
        TransitionCase {
            label: "Closed + claim_auction → Claimed",
            mode,
            from: AuctionStatus::Closed,
            entrypoint: Entrypoint::ClaimAuction,
            expect_ok: true,
            expected_error: None,
            expected_to: Some(AuctionStatus::Claimed),
            qualifying_bid: 0,
        },
        TransitionCase {
            label: "Open + claim_auction → AuctionNotClosed",
            mode,
            from: AuctionStatus::Open,
            entrypoint: Entrypoint::ClaimAuction,
            expect_ok: false,
            expected_error: Some(AuctionError::AuctionNotClosed),
            expected_to: None,
            qualifying_bid: 0,
        },
        TransitionCase {
            label: "Closed + place_bid → AuctionNotOpen",
            mode,
            from: AuctionStatus::Closed,
            entrypoint: Entrypoint::PlaceBid,
            expect_ok: false,
            expected_error: Some(AuctionError::AuctionNotOpen),
            expected_to: None,
            qualifying_bid: 200,
        },
        TransitionCase {
            label: "Closed + close_auction → AuctionNotOpen",
            mode,
            from: AuctionStatus::Closed,
            entrypoint: Entrypoint::CloseAuction,
            expect_ok: false,
            expected_error: Some(AuctionError::AuctionNotOpen),
            expected_to: None,
            qualifying_bid: 0,
        },
        TransitionCase {
            label: "Claimed + place_bid → AuctionNotOpen",
            mode,
            from: AuctionStatus::Claimed,
            entrypoint: Entrypoint::PlaceBid,
            expect_ok: false,
            expected_error: Some(AuctionError::AuctionNotOpen),
            expected_to: None,
            qualifying_bid: 200,
        },
        TransitionCase {
            label: "Claimed + close_auction → AlreadyClaimed",
            mode,
            from: AuctionStatus::Claimed,
            entrypoint: Entrypoint::CloseAuction,
            expect_ok: false,
            expected_error: Some(AuctionError::AlreadyClaimed),
            expected_to: None,
            qualifying_bid: 0,
        },
        TransitionCase {
            label: "Claimed + claim_auction → AuctionNotClosed",
            mode,
            from: AuctionStatus::Claimed,
            entrypoint: Entrypoint::ClaimAuction,
            expect_ok: false,
            expected_error: Some(AuctionError::AuctionNotClosed),
            expected_to: None,
            qualifying_bid: 0,
        },
    ]
}

fn read_status(env: &Env, contract_id: &Address, auction_id: &Symbol) -> AuctionStatus {
    let state: AuctionState = env
        .as_contract(contract_id, || env.storage().persistent().get(auction_id))
        .expect("auction state must exist");
    state.status
}

fn init_auction(client: &AuctionClient<'_>, auction_id: &Symbol, mode: AuctionMode) {
    match mode {
        AuctionMode::English => {
            client.init_auction(
                auction_id,
                &AuctionMode::English,
                &0,
                &u64::MAX,
                &1_i128,
                &0_u32,
                &None,
        &None,
        &gateway_auction::DutchAuctionDecay::None,
        &None,
);
        }
        AuctionMode::Dutch => {
            client.init_auction(
                auction_id,
                &AuctionMode::Dutch,
                &1_000,
                &2_000,
                &50_i128,
                &0_u32,
                &Some(500_i128),
                &Some(100_i128),
                &None,
                &None,
            );
            client.env.ledger().with_mut(|li| li.timestamp = 1_000);
        }
    }
}

/// Seed an auction in `from` using only legal setup paths for `mode`.
fn setup_auction(mode: AuctionMode, from: AuctionStatus) -> (Env, Address, Symbol, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);
    let winner = Address::generate(&env);
    let auction_id = Symbol::new(&env, "transition_matrix");

    client.init_auction(
        &auction_id,
        &AuctionMode::English,
        &0,
        &u64::MAX,
        &1_i128,
        &0_u32,
        &None,
        &None,
        &DutchAuctionDecay::None,
        &None,
    );

    match (mode, from.clone()) {
        (_, AuctionStatus::Open) => {}
        (AuctionMode::English, AuctionStatus::Closed) => {
            client.place_bid(&auction_id, &winner, &100_i128);
            client.close_auction(&auction_id);
        }
        (AuctionMode::English, AuctionStatus::Claimed) => {
            client.place_bid(&auction_id, &winner, &100_i128);
            client.close_auction(&auction_id);
            client.claim_auction(&auction_id);
        }
        (AuctionMode::Dutch, AuctionStatus::Closed) => {
            client.place_bid(&auction_id, &winner, &500_i128);
        }
        (AuctionMode::Dutch, AuctionStatus::Claimed) => {
            client.place_bid(&auction_id, &winner, &500_i128);
            client.claim_auction(&auction_id);
        }
    }

    assert_eq!(
        read_status(&env, &contract_id, &auction_id),
        from,
        "setup must land in the requested starting status"
    );

    (env, contract_id, auction_id, winner)
}

fn invoke_entrypoint(
    case: &TransitionCase,
    client: &AuctionClient<'_>,
    auction_id: &Symbol,
) -> Result<(), soroban_sdk::Error> {
    match case.entrypoint {
        Entrypoint::PlaceBid => {
            let bidder = Address::generate(&client.env);
            client
                .try_place_bid(auction_id, &bidder, &case.qualifying_bid)
                .map(|_| ())
                .map_err(|e| e.unwrap())
        }
        Entrypoint::CloseAuction => client
            .try_close_auction(auction_id)
            .map(|_| ())
            .map_err(|e| e.unwrap()),
        Entrypoint::ClaimAuction => client
            .try_claim_auction(auction_id)
            .map(|_| ())
            .map_err(|e| e.unwrap()),
    }
}

fn run_matrix(mode: AuctionMode) {
    for case in transition_matrix(mode) {
        let from = case.from.clone();
        let (env, contract_id, auction_id, _winner) = setup_auction(mode.clone(), from);
        let client = AuctionClient::new(&env, &contract_id);
        let status_before = read_status(&env, &contract_id, &auction_id);

        let result = invoke_entrypoint(&case, &client, &auction_id);

        if case.expect_ok {
            assert!(
                result.is_ok(),
                "{:?} {}: expected success, got {:?}",
                mode,
                case.label,
                result
            );
            let status_after = read_status(&env, &contract_id, &auction_id);
            assert_eq!(
                status_after,
                case.expected_to
                    .expect("legal case must declare expected_to"),
                "{:?} {}: unexpected post-transition status",
                mode,
                case.label
            );
        } else {
            assert!(
                result.is_err(),
                "{:?} {}: expected contract error revert",
                mode,
                case.label
            );
            let err = result.unwrap_err();
            assert_eq!(
                err,
                case.expected_error
                    .expect("illegal case must declare expected_error")
                    .into(),
                "{:?} {}: wrong AuctionError discriminant",
                mode,
                case.label
            );
            assert_eq!(
                read_status(&env, &contract_id, &auction_id),
                status_before,
                "{:?} {}: status must be unchanged after illegal transition",
                mode,
                case.label
            );
        }
    }
}

#[test]
fn english_auction_status_transition_matrix() {
    run_matrix(AuctionMode::English);
}

#[test]
fn dutch_auction_status_transition_matrix() {
    run_matrix(AuctionMode::Dutch);
}

#[test]
fn illegal_transition_count_is_six_per_mode() {
    for mode in [AuctionMode::English, AuctionMode::Dutch] {
        let illegal = transition_matrix(mode.clone())
            .into_iter()
            .filter(|c| !c.expect_ok)
            .count();
        assert_eq!(
            illegal, 6,
            "{mode:?}: matrix must cover exactly six illegal pairs"
        );
    }
}

#[test]
fn legal_transition_count_is_three_per_mode() {
    for mode in [AuctionMode::English, AuctionMode::Dutch] {
        let legal = transition_matrix(mode.clone())
            .into_iter()
            .filter(|c| c.expect_ok)
            .count();
        assert_eq!(
            legal, 3,
            "{mode:?}: matrix must cover exactly three legal pairs"
        );
    }
}
