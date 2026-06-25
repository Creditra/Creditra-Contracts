//! Exhaustive `AuctionStatus` transition matrix (Issue #489).
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
//! | From    | `place_bid`     | `close_auction`      | `claim_auction`        |
//! |---------|-----------------|----------------------|------------------------|
//! | Open    | ✓ (stays Open)  | ✓ → Closed           | ✗ `AuctionNotClosed=9` |
//! | Closed  | ✗ `NotOpen=8`   | ✗ `NotOpen=8`        | ✓ → Claimed            |
//! | Claimed | ✗ `NotOpen=8`   | ✗ `AlreadyClaimed=2` | ✗ `NotClosed=9`        |
//!
//! Six illegal pairs; three legal pairs. Every illegal pair must revert with the
//! documented `AuctionError` discriminant (not a generic host panic).
//!
//! # Running
//!
//! ```bash
//! cargo test -p gateway-auction --test status_transitions
//! ```

use gateway_auction::{Auction, AuctionClient, AuctionError, AuctionMode, AuctionState, AuctionStatus};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Symbol};

/// Entrypoints that can attempt an `AuctionStatus` transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Entrypoint {
    PlaceBid,
    CloseAuction,
    ClaimAuction,
}

/// One row of the status × entrypoint matrix.
#[derive(Clone, Debug)]
struct TransitionCase {
    label: &'static str,
    from: AuctionStatus,
    entrypoint: Entrypoint,
    expect_ok: bool,
    expected_error: Option<AuctionError>,
    expected_to: Option<AuctionStatus>,
}

fn transition_matrix() -> Vec<TransitionCase> {
    vec![
        // ── Legal transitions (3) ────────────────────────────────────────────
        TransitionCase {
            label: "Open + place_bid → Open",
            from: AuctionStatus::Open,
            entrypoint: Entrypoint::PlaceBid,
            expect_ok: true,
            expected_error: None,
            expected_to: Some(AuctionStatus::Open),
        },
        TransitionCase {
            label: "Open + close_auction → Closed",
            from: AuctionStatus::Open,
            entrypoint: Entrypoint::CloseAuction,
            expect_ok: true,
            expected_error: None,
            expected_to: Some(AuctionStatus::Closed),
        },
        TransitionCase {
            label: "Closed + claim_auction → Claimed",
            from: AuctionStatus::Closed,
            entrypoint: Entrypoint::ClaimAuction,
            expect_ok: true,
            expected_error: None,
            expected_to: Some(AuctionStatus::Claimed),
        },
        // ── Illegal transitions (6) ────────────────────────────────────────
        TransitionCase {
            label: "Open + claim_auction → AuctionNotClosed",
            from: AuctionStatus::Open,
            entrypoint: Entrypoint::ClaimAuction,
            expect_ok: false,
            expected_error: Some(AuctionError::AuctionNotClosed),
            expected_to: None,
        },
        TransitionCase {
            label: "Closed + place_bid → AuctionNotOpen",
            from: AuctionStatus::Closed,
            entrypoint: Entrypoint::PlaceBid,
            expect_ok: false,
            expected_error: Some(AuctionError::AuctionNotOpen),
            expected_to: None,
        },
        TransitionCase {
            label: "Closed + close_auction → AuctionNotOpen",
            from: AuctionStatus::Closed,
            entrypoint: Entrypoint::CloseAuction,
            expect_ok: false,
            expected_error: Some(AuctionError::AuctionNotOpen),
            expected_to: None,
        },
        TransitionCase {
            label: "Claimed + place_bid → AuctionNotOpen",
            from: AuctionStatus::Claimed,
            entrypoint: Entrypoint::PlaceBid,
            expect_ok: false,
            expected_error: Some(AuctionError::AuctionNotOpen),
            expected_to: None,
        },
        TransitionCase {
            label: "Claimed + close_auction → AlreadyClaimed",
            from: AuctionStatus::Claimed,
            entrypoint: Entrypoint::CloseAuction,
            expect_ok: false,
            expected_error: Some(AuctionError::AlreadyClaimed),
            expected_to: None,
        },
        TransitionCase {
            label: "Claimed + claim_auction → AuctionNotClosed",
            from: AuctionStatus::Claimed,
            entrypoint: Entrypoint::ClaimAuction,
            expect_ok: false,
            expected_error: Some(AuctionError::AuctionNotClosed),
            expected_to: None,
        },
    ]
}

fn read_status(env: &Env, contract_id: &Address, auction_id: &Symbol) -> AuctionStatus {
    let state: AuctionState = env
        .as_contract(contract_id, || env.storage().persistent().get(auction_id))
        .expect("auction state must exist");
    state.status
}

/// Seed an auction in `from` using only legal setup paths.
fn setup_auction(
    from: AuctionStatus,
) -> (Env, Address, Symbol, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Auction, ());
    let client = AuctionClient::new(&env, &contract_id);
    let winner = Address::generate(&env);
    let auction_id = Symbol::new(&env, "status_matrix");

    client.init_auction(
        &auction_id,
        &AuctionMode::English,
        &0,
        &u64::MAX,
        &1_i128,
        &0_u32,
        &None,
        &None,
    );

    match from {
        AuctionStatus::Open => {}
        AuctionStatus::Closed => {
            client.place_bid(&auction_id, &winner, &100_i128);
            client.close_auction(&auction_id);
        }
        AuctionStatus::Claimed => {
            client.place_bid(&auction_id, &winner, &100_i128);
            client.close_auction(&auction_id);
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
                .try_place_bid(auction_id, &bidder, &200_i128)
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

#[test]
fn auction_status_transition_matrix() {
    for case in transition_matrix() {
        let from = case.from.clone();
        let (env, contract_id, auction_id, _winner) = setup_auction(from);
        let client = AuctionClient::new(&env, &contract_id);
        let status_before = read_status(&env, &contract_id, &auction_id);

        let result = invoke_entrypoint(&case, &client, &auction_id);

        if case.expect_ok {
            assert!(
                result.is_ok(),
                "{}: expected success, got {:?}",
                case.label,
                result
            );
            let status_after = read_status(&env, &contract_id, &auction_id);
            assert_eq!(
                status_after,
                case.expected_to.expect("legal case must declare expected_to"),
                "{}: unexpected post-transition status",
                case.label
            );
        } else {
            assert!(
                result.is_err(),
                "{}: expected contract error revert",
                case.label
            );
            let err = result.unwrap_err();
            assert_eq!(
                err,
                case.expected_error
                    .expect("illegal case must declare expected_error")
                    .into(),
                "{}: wrong AuctionError discriminant",
                case.label
            );
            // Illegal transitions must not mutate stored status.
            assert_eq!(
                read_status(&env, &contract_id, &auction_id),
                status_before,
                "{}: status must be unchanged after illegal transition",
                case.label
            );
        }
    }
}

#[test]
fn illegal_transition_count_is_six() {
    let illegal = transition_matrix()
        .into_iter()
        .filter(|c| !c.expect_ok)
        .count();
    assert_eq!(illegal, 6, "matrix must cover exactly six illegal pairs");
}

#[test]
fn legal_transition_count_is_three() {
    let legal = transition_matrix()
        .into_iter()
        .filter(|c| c.expect_ok)
        .count();
    assert_eq!(legal, 3, "matrix must cover exactly three legal pairs");
}
