// SPDX-License-Identifier: MIT

//! ContractError stability tests for the gateway-auction (v7) subsystem.
//!
//! # What
//!
//! Focused CI guard for the error discriminants used by the v7 auction engine
//! (`gateway_auction::AuctionError`). Any assertion failure means a
//! discriminant was accidentally changed — breaking deployed SDK clients and
//! indexers that match on error codes.
//!
//! # Scope (v7 auction surface)
//!
//! All 14 `AuctionError` variants are pinned:
//!
//! | Discriminant | Variant | Typical trigger |
//! |---|---|---|
//! | 1  | `NotWinner` | Claimant is not the winning bidder (unused in current code) |
//! | 2  | `AlreadyClaimed` | `close_auction` on a `Claimed` auction |
//! | 3  | `NotClosed` | `settle_default_liquidation` on an `Open` auction |
//! | 4  | `NoFactoryContract` | Any factory-gated entrypoint before `set_factory_contract` |
//! | 5  | `Unauthorized` | `settle_default_liquidation` with wrong `credit_contract` |
//! | 6  | `InvalidState` | `claim_auction` without a configured `bid_token` |
//! | 7  | `BidTooLow` | `place_bid` with zero / below-threshold amount |
//! | 8  | `AuctionNotOpen` | `place_bid` or `close_auction` on a non-`Open` auction |
//! | 9  | `AuctionNotClosed` | `claim_auction` on an `Open` auction |
//! | 10 | `Reentrancy` | Reentrant token callback detected by the reentrancy guard |
//! | 11 | `NoWinner` | `claim_auction` when no bid was ever placed |
//! | 12 | `NotFound` | Any entrypoint with a non-existent `auction_id` |
//! | 13 | `AlreadySettled` | Second call to `settle_default_liquidation` for the same auction |
//! | 14 | `GracePeriodActive` | `place_bid` before the liquidation grace window has elapsed |
//!
//! # Rules
//! - Never change an existing assertion value.
//! - If a new auction error variant is added, append it with the next
//!   available integer **and** add corresponding assertions here.
//! - Integration tests MUST verify the raw discriminant (e.g. `"#12"`) is
//!   encoded in the panic payload — never match on variant names alone.
//!
//! # See also
//! - `src/errors.rs` — the `AuctionError` enum definition.
//! - `tests/panic_with_error.rs` — per-entrypoint `try_`-style error matching.
//! - `docs/PROTOCOL_SPEC.md` — documented error-code table.

use gateway_auction::{Auction, AuctionClient, AuctionError, AuctionMode};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env, Symbol};

// ═══════════════════════════════════════════════════════════════════════════
// Section 1 — Discriminant stability pins (v7 auction error surface)
// ═══════════════════════════════════════════════════════════════════════════

/// Pin every discriminant in the v7 auction error surface.
///
/// Values below are **permanent** — they are embedded in deployed SDKs and
/// on-chain indexer matchers. If any assertion fails, inspect
/// `gateway_auction::AuctionError` for an accidental reorder / renumber of
/// the `#[repr(u32)]` enum.
#[test]
fn gateway_auction_v7_error_discriminants_are_pinned() {
    assert_eq!(AuctionError::NotWinner as u32, 1);
    assert_eq!(AuctionError::AlreadyClaimed as u32, 2);
    assert_eq!(AuctionError::NotClosed as u32, 3);
    assert_eq!(AuctionError::NoFactoryContract as u32, 4);
    assert_eq!(AuctionError::Unauthorized as u32, 5);
    assert_eq!(AuctionError::InvalidState as u32, 6);
    assert_eq!(AuctionError::BidTooLow as u32, 7);
    assert_eq!(AuctionError::AuctionNotOpen as u32, 8);
    assert_eq!(AuctionError::AuctionNotClosed as u32, 9);
    assert_eq!(AuctionError::Reentrancy as u32, 10);
    assert_eq!(AuctionError::NoWinner as u32, 11);
    assert_eq!(AuctionError::NotFound as u32, 12);
    assert_eq!(AuctionError::AlreadySettled as u32, 13);
    assert_eq!(AuctionError::GracePeriodActive as u32, 14);
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 2 — Duplicate-free + variant-count sanity (v7 subset)
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that no two v7 auction variants share a discriminant.
#[test]
fn gateway_auction_v7_subset_has_no_duplicate_discriminants() {
    use std::collections::HashSet;

    let codes: Vec<u32> = vec![
        AuctionError::NotWinner as u32,
        AuctionError::AlreadyClaimed as u32,
        AuctionError::NotClosed as u32,
        AuctionError::NoFactoryContract as u32,
        AuctionError::Unauthorized as u32,
        AuctionError::InvalidState as u32,
        AuctionError::BidTooLow as u32,
        AuctionError::AuctionNotOpen as u32,
        AuctionError::AuctionNotClosed as u32,
        AuctionError::Reentrancy as u32,
        AuctionError::NoWinner as u32,
        AuctionError::NotFound as u32,
        AuctionError::AlreadySettled as u32,
        AuctionError::GracePeriodActive as u32,
    ];

    let unique: HashSet<u32> = codes.iter().cloned().collect();
    assert_eq!(
        codes.len(),
        unique.len(),
        "Duplicate discriminants in the v7 auction error surface — inspect errors.rs"
    );
}

/// Known count: 14 variants in the v7 auction surface (pinned above).
///
/// If this assertion fails, a new auction variant was added to or removed
/// from the `AuctionError` enum — update the count AND add/remove the
/// corresponding pinning assertions in
/// `gateway_auction_v7_error_discriminants_are_pinned`.
#[test]
fn gateway_auction_v7_subset_variant_count_is_known() {
    const EXPECTED_VARIANT_COUNT: usize = 14;

    let codes = [
        AuctionError::NotWinner as u32,
        AuctionError::AlreadyClaimed as u32,
        AuctionError::NotClosed as u32,
        AuctionError::NoFactoryContract as u32,
        AuctionError::Unauthorized as u32,
        AuctionError::InvalidState as u32,
        AuctionError::BidTooLow as u32,
        AuctionError::AuctionNotOpen as u32,
        AuctionError::AuctionNotClosed as u32,
        AuctionError::Reentrancy as u32,
        AuctionError::NoWinner as u32,
        AuctionError::NotFound as u32,
        AuctionError::AlreadySettled as u32,
        AuctionError::GracePeriodActive as u32,
    ];

    assert_eq!(
        codes.len(),
        EXPECTED_VARIANT_COUNT,
        "v7 auction surface variant count changed — pin new assertions and update EXPECTED_VARIANT_COUNT"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 3 — Integration: runtime error paths return the pinned discriminant
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod integration {
    use super::*;
    use soroban_sdk::token::StellarAssetClient;

    // ── Helpers ──────────────────────────────────────────────────────────

    /// Extract the raw Soroban error string from a caught panic payload.
    ///
    /// Soroban encodes contract errors as `"Error(Contract, #<discriminant>)"`
    /// inside the panic message. We string-match because the opaque payload
    /// does not implement `PartialEq` across Soroban versions.
    fn extract_error_str(payload: &Box<dyn std::any::Any + Send>) -> String {
        if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else if let Some(s) = payload.downcast_ref::<&str>() {
            s.to_string()
        } else {
            String::new()
        }
    }

    /// Deploy, register a factory, and initialize a long-lived English auction.
    fn setup_english_auction(env: &Env) -> (Address, Address, Symbol) {
        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(env, &contract_id);
        let factory = Address::generate(env);
        client.set_factory_contract(&factory);

        let auction_id = Symbol::new(env, "err_stab");
        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0_u64,
            &u64::MAX,
            &50_i128,
            &0_u32,
            &None,
            &None,
            &gateway_auction::DutchAuctionDecay::None,
            &None,
        );
        (contract_id, factory, auction_id)
    }

    /// Set a `bid_token` in instance storage for the given contract.
    fn configure_bid_token(env: &Env, contract_id: &Address) {
        let token_admin = Address::generate(env);
        let token_id = env.register_stellar_asset_contract_v2(token_admin);
        let bid_token = token_id.address();
        env.as_contract(contract_id, || {
            env.storage()
                .instance()
                .set(&Symbol::new(env, "bid_token"), &bid_token);
        });
    }

    // ── Test 3.1 — NotFound (12) via place_bid on missing auction ────────

    /// `place_bid` on a non-existent `auction_id` MUST revert with
    /// `AuctionError::NotFound` (discriminant 12).
    #[test]
    fn place_bid_missing_id_returns_not_found_code_12() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.place_bid(
                &Symbol::new(&env, "ghost"),
                &Address::generate(&env),
                &100_i128,
            );
        }));
        assert!(result.is_err(), "expected revert for missing auction");

        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#12"),
            "expected NotFound (#12), got: {:?}",
            err_str
        );
        // Sanity: must NOT be mistaken for any other code.
        assert!(!err_str.contains("#7"), "must not be BidTooLow");
    }

    // ── Test 3.2 — BidTooLow (7) via place_bid with zero amount ──────────

    /// `place_bid` with amount <= 0 MUST revert with
    /// `AuctionError::BidTooLow` (discriminant 7).
    #[test]
    fn place_bid_zero_amount_returns_bid_too_low_code_7() {
        let env = Env::default();
        env.mock_all_auths();

        let (_contract_id, _factory, auction_id) = setup_english_auction(&env);
        let client = AuctionClient::new(&env, &_contract_id);

        for amount in [0_i128, -1_i128] {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                client.place_bid(&auction_id, &Address::generate(&env), &amount);
            }));
            assert!(result.is_err(), "amount {amount} must revert");

            let err_str = extract_error_str(&result.unwrap_err());
            assert!(
                err_str.contains("#7"),
                "amount {amount}: expected BidTooLow (#7), got: {:?}",
                err_str
            );
        }
    }

    // ── Test 3.3 — AuctionNotOpen (8) via place_bid after end_time ───────

    /// `place_bid` after the auction's `end_time` has passed MUST revert
    /// with `AuctionError::AuctionNotOpen` (discriminant 8).
    #[test]
    fn place_bid_after_end_time_returns_not_open_code_8() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(2000);

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let factory = Address::generate(&env);
        client.set_factory_contract(&factory);

        let auction_id = Symbol::new(&env, "expired");
        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0_u64,
            &1000_u64,
            &50_i128,
            &0_u32,
            &None,
            &None,
            &gateway_auction::DutchAuctionDecay::None,
            &None,
        );

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.place_bid(&auction_id, &Address::generate(&env), &100_i128);
        }));
        assert!(result.is_err(), "expected revert for expired auction");

        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#8"),
            "expected AuctionNotOpen (#8), got: {:?}",
            err_str
        );
    }

    // ── Test 3.4 — GracePeriodActive (14) via place_bid during grace window ──

    /// `place_bid` before the liquidation grace window has elapsed MUST
    /// revert with `AuctionError::GracePeriodActive` (discriminant 14).
    #[test]
    fn place_bid_during_grace_window_returns_grace_period_active_code_14() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(500);

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let factory = Address::generate(&env);
        client.set_factory_contract(&factory);

        // Set a 1000-second grace window.

        let auction_id = Symbol::new(&env, "grace_test");
        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0_u64,
            &u64::MAX,
            &50_i128,
            &0_u32,
            &None,
            &None,
            &gateway_auction::DutchAuctionDecay::None,
            &None,
        );

        // current timestamp (500) < start_time (0) + grace_window (1000)
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.place_bid(&auction_id, &Address::generate(&env), &100_i128);
        }));
        assert!(result.is_err(), "expected revert during grace window");

        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#14"),
            "expected GracePeriodActive (#14), got: {:?}",
            err_str
        );
    }

    // ── Test 3.5 — AlreadyClaimed (2) via close_auction on claimed auction ──

    /// `close_auction` on an auction that has already been `Claimed` MUST
    /// revert with `AuctionError::AlreadyClaimed` (discriminant 2).
    ///
    /// To reach the `Claimed` status the bidder must hold sufficient tokens:
    /// `place_bid` transfers from bidder → contract, then `claim_auction`
    /// transfers from contract → winner.
    fn setup_claimed_auction(env: &Env, contract_id: &Address, auction_id: &Symbol) -> Address {
        let client = AuctionClient::new(env, contract_id);
        let bidder = Address::generate(env);

        // Create a token, mint to bidder, configure as bid_token.
        let token_admin = Address::generate(env);
        let token_id = env.register_stellar_asset_contract_v2(token_admin);
        let bid_token = token_id.address();
        let sac = StellarAssetClient::new(env, &bid_token);
        sac.mint(&bidder, &1000_i128);
        env.as_contract(contract_id, || {
            env.storage()
                .instance()
                .set(&Symbol::new(env, "bid_token"), &bid_token);
        });

        client.place_bid(auction_id, &bidder, &100_i128);
        client.close_auction(auction_id);
        client.claim_auction(auction_id);
        bidder
    }

    #[test]
    fn close_auction_on_claimed_returns_already_claimed_code_2() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract_id, _factory, auction_id) = setup_english_auction(&env);
        let client = AuctionClient::new(&env, &contract_id);

        setup_claimed_auction(&env, &contract_id, &auction_id);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.close_auction(&auction_id);
        }));
        assert!(result.is_err(), "expected revert on claimed auction");

        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#2"),
            "expected AlreadyClaimed (#2), got: {:?}",
            err_str
        );
    }

    // ── Test 3.6 — NotClosed (3) via settle on open auction ──────────────

    /// `settle_default_liquidation` on an `Open` (not yet closed) auction
    /// MUST revert with `AuctionError::NotClosed` (discriminant 3).
    #[test]
    fn settle_open_auction_returns_not_closed_code_3() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract_id, factory, auction_id) = setup_english_auction(&env);
        let client = AuctionClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);

        // Auction is Open — do NOT close it.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.settle_default_liquidation(&auction_id, &factory, &borrower);
        }));
        assert!(result.is_err(), "expected revert for open auction settle");

        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#3"),
            "expected NotClosed (#3), got: {:?}",
            err_str
        );
    }

    // ── Test 3.7 — NoFactoryContract (4) via close_auction without factory ──

    /// `close_auction` without a registered factory contract MUST revert
    /// with `AuctionError::NoFactoryContract` (discriminant 4).
    #[test]
    fn close_auction_without_factory_returns_no_factory_code_4() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.close_auction(&Symbol::new(&env, "nofac"));
        }));
        assert!(result.is_err(), "expected revert without factory");

        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#4"),
            "expected NoFactoryContract (#4), got: {:?}",
            err_str
        );
    }

    // ── Test 3.8 — Unauthorized (5) via settle with wrong credit_contract ──

    /// `settle_default_liquidation` with a `credit_contract` that does not
    /// match the registered factory MUST revert with
    /// `AuctionError::Unauthorized` (discriminant 5).
    #[test]
    fn settle_with_wrong_credit_contract_returns_unauthorized_code_5() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract_id, _factory, auction_id) = setup_english_auction(&env);
        let client = AuctionClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);
        let bidder = Address::generate(&env);
        let wrong_contract = Address::generate(&env);

        client.place_bid(&auction_id, &bidder, &100_i128);
        client.close_auction(&auction_id);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.settle_default_liquidation(&auction_id, &wrong_contract, &borrower);
        }));
        assert!(
            result.is_err(),
            "expected revert with wrong credit contract"
        );

        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#5"),
            "expected Unauthorized (#5), got: {:?}",
            err_str
        );
    }

    // ── Test 3.9 — InvalidState (6) via claim_auction without bid_token ──

    /// `claim_auction` without a configured `bid_token` MUST revert with
    /// `AuctionError::InvalidState` (discriminant 6).
    #[test]
    fn claim_without_bid_token_returns_invalid_state_code_6() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract_id, _factory, auction_id) = setup_english_auction(&env);
        let client = AuctionClient::new(&env, &contract_id);
        let bidder = Address::generate(&env);

        // No bid_token configured.
        client.place_bid(&auction_id, &bidder, &100_i128);
        client.close_auction(&auction_id);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.claim_auction(&auction_id);
        }));
        assert!(result.is_err(), "expected revert without bid_token");

        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#6"),
            "expected InvalidState (#6), got: {:?}",
            err_str
        );
    }

    // ── Test 3.10 — AuctionNotClosed (9) via claim_auction on open auction ──

    /// `claim_auction` on an `Open` auction MUST revert with
    /// `AuctionError::AuctionNotClosed` (discriminant 9).
    #[test]
    fn claim_open_auction_returns_not_closed_code_9() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract_id, _factory, auction_id) = setup_english_auction(&env);
        let client = AuctionClient::new(&env, &contract_id);

        // Auction is still Open.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.claim_auction(&auction_id);
        }));
        assert!(result.is_err(), "expected revert for open auction claim");

        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#9"),
            "expected AuctionNotClosed (#9), got: {:?}",
            err_str
        );
    }

    // ── Test 3.11 — Reentrancy (10) via set_reentrancy_guard pre-set ─────

    /// When the reentrancy guard is already set, the next guarded token
    /// interaction MUST revert with `AuctionError::Reentrancy` (10).
    #[test]
    fn reentrancy_guard_active_returns_reentrancy_code_10() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract_id, _factory, auction_id) = setup_english_auction(&env);
        let client = AuctionClient::new(&env, &contract_id);

        configure_bid_token(&env, &contract_id);

        // Manually set the reentrancy flag to simulate a prior in-flight
        // token transfer that never cleared the guard.
        env.as_contract(&contract_id, || {
            env.storage()
                .instance()
                .set(&Symbol::new(&env, "reentrancy"), &true);
        });

        // Any place_bid with a bid_token will call set_reentrancy_guard
        // and detect the pre-existing flag.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.place_bid(&auction_id, &Address::generate(&env), &100_i128);
        }));
        assert!(result.is_err(), "expected revert when reentrancy guard set");

        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#10"),
            "expected Reentrancy (#10), got: {:?}",
            err_str
        );
    }

    // ── Test 3.12 — NoWinner (11) via claim_auction with no bids ─────────

    /// `claim_auction` on a `Closed` auction that received zero bids MUST
    /// revert with `AuctionError::NoWinner` (discriminant 11).
    #[test]
    fn claim_no_bids_returns_no_winner_code_11() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract_id, _factory, auction_id) = setup_english_auction(&env);
        let client = AuctionClient::new(&env, &contract_id);

        // Close without any bids.
        client.close_auction(&auction_id);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.claim_auction(&auction_id);
        }));
        assert!(result.is_err(), "expected revert with no winner");

        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#11"),
            "expected NoWinner (#11), got: {:?}",
            err_str
        );
    }

    // ── Test 3.13 — AlreadySettled (13) via double settle ────────────────

    /// A second `settle_default_liquidation` call for the same auction MUST
    /// revert with `AuctionError::AlreadySettled` (discriminant 13).
    #[test]
    fn double_settle_returns_already_settled_code_13() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract_id, factory, auction_id) = setup_english_auction(&env);
        let client = AuctionClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);
        let bidder = Address::generate(&env);

        client.place_bid(&auction_id, &bidder, &100_i128);
        client.close_auction(&auction_id);

        // First settle succeeds.
        let _ = client.settle_default_liquidation(&auction_id, &factory, &borrower);

        // Second settle MUST fail.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.settle_default_liquidation(&auction_id, &factory, &borrower);
        }));
        assert!(result.is_err(), "expected revert for double settle");

        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#13"),
            "expected AlreadySettled (#13), got: {:?}",
            err_str
        );
    }

    // ── Test 3.14 — BidTooLow (7) via bid below min_bid ──────────────────

    /// `place_bid` with an amount strictly below `min_bid` MUST revert with
    /// `AuctionError::BidTooLow` (discriminant 7).
    #[test]
    fn place_bid_below_min_bid_returns_bid_too_low_code_7() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let factory = Address::generate(&env);
        client.set_factory_contract(&factory);

        let auction_id = Symbol::new(&env, "min_bid_test");
        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0_u64,
            &u64::MAX,
            &100_i128, // min_bid = 100
            &0_u32,
            &None,
            &None,
            &gateway_auction::DutchAuctionDecay::None,
            &None,
        );

        // 99 < 100 → BidTooLow
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.place_bid(&auction_id, &Address::generate(&env), &99_i128);
        }));
        assert!(result.is_err(), "expected revert for below-min bid");

        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#7"),
            "expected BidTooLow (#7) for below-min bid, got: {:?}",
            err_str
        );
    }

    // ── Test 3.15 — AuctionNotOpen (8) via close_auction on closed auction ──

    /// `close_auction` on an already-`Closed` auction MUST revert with
    /// `AuctionError::AuctionNotOpen` (discriminant 8).
    #[test]
    fn close_already_closed_returns_not_open_code_8() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract_id, _factory, auction_id) = setup_english_auction(&env);
        let client = AuctionClient::new(&env, &contract_id);
        let bidder = Address::generate(&env);

        client.place_bid(&auction_id, &bidder, &100_i128);
        client.close_auction(&auction_id);

        // Second close must fail.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.close_auction(&auction_id);
        }));
        assert!(result.is_err(), "expected revert for double close");

        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#8"),
            "expected AuctionNotOpen (#8), got: {:?}",
            err_str
        );
    }

    // ── Test 3.16 — NotFound (12) via close_auction on missing ID ────────

    /// `close_auction` on a non-existent `auction_id` MUST revert with
    /// `AuctionError::NotFound` (discriminant 12), even when a factory is
    /// registered.
    #[test]
    fn close_auction_missing_id_returns_not_found_code_12() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let factory = Address::generate(&env);
        client.set_factory_contract(&factory);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.close_auction(&Symbol::new(&env, "ghost_close"));
        }));
        assert!(result.is_err(), "expected revert for missing auction");

        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#12"),
            "expected NotFound (#12), got: {:?}",
            err_str
        );
    }

    // ── Test 3.17 — AlreadySettled (13) via claim_auction after settle ───

    /// `claim_auction` on a settled auction MUST revert with
    /// `AuctionError::AlreadySettled` (discriminant 13).
    #[test]
    fn claim_after_settle_returns_already_settled_code_13() {
        let env = Env::default();
        env.mock_all_auths();

        let (contract_id, factory, auction_id) = setup_english_auction(&env);
        let client = AuctionClient::new(&env, &contract_id);
        let borrower = Address::generate(&env);
        let bidder = Address::generate(&env);

        client.place_bid(&auction_id, &bidder, &100_i128);
        client.close_auction(&auction_id);
        client.settle_default_liquidation(&auction_id, &factory, &borrower);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.claim_auction(&auction_id);
        }));
        assert!(result.is_err(), "expected revert for claim after settle");

        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#13"),
            "expected AlreadySettled (#13), got: {:?}",
            err_str
        );
    }

    // ── Test 3.18 — GracePeriodActive (14) via Dutch auction grace window ──

    /// Dutch mode + grace window: bid placed inside the window MUST revert
    /// with `GracePeriodActive` (14).
    #[test]
    fn dutch_bid_during_grace_window_returns_grace_period_active_code_14() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(100);

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let factory = Address::generate(&env);
        client.set_factory_contract(&factory);

        let auction_id = Symbol::new(&env, "dutch_grace");
        client.init_auction(
            &auction_id,
            &AuctionMode::Dutch,
            &0_u64,    // start_time
            &2000_u64, // end_time
            &50_i128,  // min_bid
            &0_u32,
            &Some(500_i128), // dutch_start_price
            &Some(100_i128), // dutch_floor_price
            &None,
            &None,
        );

        // timestamp 100 < start_time 0 + grace 500
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.place_bid(&auction_id, &Address::generate(&env), &500_i128);
        }));
        assert!(result.is_err(), "expected revert during grace window");

        let err_str = extract_error_str(&result.unwrap_err());
        assert!(
            err_str.contains("#14"),
            "expected GracePeriodActive (#14), got: {:?}",
            err_str
        );
    }
}
