#[cfg(test)]
mod tests {
    extern crate std;
    use super::super::*;
    use core::ops::Range;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::vec::Vec;

    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::testutils::Events as _;
    use soroban_sdk::testutils::Ledger as _;
    use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
    use soroban_sdk::{Address, Env, Symbol, TryFromVal, TryIntoVal};

    const REFUND_TOPIC: &str = "BID_RFDN";
    const SETTLEMENT_TOPIC: &str = "LIQ_SETL";
    const AUCTION_ID: &str = "inv_auc";
    const FUZZ_STEPS: usize = 64;
    const MAX_INCREMENT: u64 = 500;

    fn advance_ledgers(env: &Env, ledgers: u32) {
        env.ledger().with_mut(|li| {
            li.sequence_number += ledgers;
            li.timestamp += (ledgers as u64) * 5;
        });
    }

    fn next_u64(state: &mut u64) -> u64 {
        let mut x = *state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *state = x;
        x
    }

    fn pick_index(seed: &mut u64, range: Range<usize>) -> usize {
        let len = range.end - range.start;
        range.start + (next_u64(seed) as usize % len)
    }

    fn next_amount_above(seed: &mut u64, current: i128) -> i128 {
        current + i128::from((next_u64(seed) % MAX_INCREMENT) + 1)
    }

    fn refunded_events(env: &Env) -> Vec<events::BidRefundedEvent> {
        let mut output = Vec::new();
        for (_contract, topics, data) in env.events().all().iter() {
            let t0: Symbol = Symbol::try_from_val(env, &topics.get(0).unwrap()).unwrap();
            if t0 == Symbol::new(env, REFUND_TOPIC) {
                let event_data: events::BidRefundedEvent = data.try_into_val(env).unwrap();
                output.push(event_data);
            }
        }
        output
    }

    fn settlement_events(env: &Env) -> Vec<events::DefaultLiquidationSettlementEvent> {
        let mut output = Vec::new();
        for (_contract, topics, data) in env.events().all().iter() {
            let t0: Symbol = Symbol::try_from_val(env, &topics.get(0).unwrap()).unwrap();
            if t0 == Symbol::new(env, SETTLEMENT_TOPIC) {
                let event_data: events::DefaultLiquidationSettlementEvent =
                    data.try_into_val(env).unwrap();
                output.push(event_data);
            }
        }
        output
    }

    #[test]
    fn bid_refunded_event_emitted_on_outbid() {
        let env = Env::default();
        env.mock_all_auths();

        let alice = Address::generate(&env);
        let bob = Address::generate(&env);

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let auction_id = Symbol::new(&env, "auc1");
        client.init_auction(&auction_id, &0, &1000, &50_i128, &0_u32); // start 0, end 1000, min 50, 0 bps

        client.place_bid(&auction_id, &alice, &100_i128);
        client.place_bid(&auction_id, &bob, &200_i128);

        let refund_events = refunded_events(&env);
        assert_eq!(refund_events.len(), 1);
        let event_data = refund_events.last().unwrap();
        assert_eq!(event_data.prev_bidder, alice);
        assert_eq!(event_data.amount, 100_i128);
    }

    #[test]
    fn fuzz_bid_sequence_invariants_deterministic() {
        let env = Env::default();
        env.mock_all_auths();

        let bidders: [Address; 5] = [
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
        ];

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let auction_id = Symbol::new(&env, AUCTION_ID);

        client.init_auction(&auction_id, &0, &u64::MAX, &1_i128, &0_u32); // long auction, min 1, 0 bps

        let mut seed: u64 = 0xdeadbeefcafebabe;
        let mut expected: Option<(Address, i128)> = None;

        for _ in 0..FUZZ_STEPS {
            let bidder_idx = pick_index(&mut seed, 0..bidders.len());
            let bidder = bidders[bidder_idx].clone();
            let amount =
                next_amount_above(&mut seed, expected.as_ref().map(|(_, a)| *a).unwrap_or(0));

            client.place_bid(&auction_id, &bidder, &amount);

            // In soroban-sdk v22, env.events() returns events from the most recent successful
            // transaction only (not cumulative). Check that this bid emitted exactly one
            // BID_RFDN event with the correct previous bidder and amount.
            if let Some((prev_addr, prev_amount)) = expected.clone() {
                let events = refunded_events(&env);
                assert_eq!(events.len(), 1, "expected one refund event per outbid");
                let evt = events.last().unwrap();
                assert_eq!(evt.prev_bidder, prev_addr);
                assert_eq!(evt.amount, prev_amount);
            }

            expected = Some((bidder.clone(), amount));

            let stored: Option<crate::types::AuctionState> =
                env.as_contract(&contract_id, || env.storage().persistent().get(&auction_id));
            assert!(stored.is_some(), "stored state must exist");
            let s = stored.unwrap();
            assert_eq!(s.highest_bidder.unwrap(), bidder);
            assert_eq!(s.highest_bid, amount);

            let invalid_bidder_idx = pick_index(&mut seed, 0..bidders.len());
            // Use try_place_bid (Result-returning variant) to avoid corrupting the Env
            // when the contract panics. catch_unwind interacts poorly with soroban's
            // RefCell-based host when the contract has already matched on stored state.
            let invalid_attempt =
                client.try_place_bid(&auction_id, &bidders[invalid_bidder_idx], &amount);
            assert!(invalid_attempt.is_err(), "equal bid unexpectedly accepted");

            let stored_after_invalid: crate::types::AuctionState = env
                .as_contract(&contract_id, || env.storage().persistent().get(&auction_id))
                .unwrap();
            assert_eq!(stored_after_invalid.highest_bidder.unwrap(), bidder);
            assert_eq!(stored_after_invalid.highest_bid, amount);
            // Note: cannot check event count here — soroban-sdk v22 resets env.events()
            // after every failed contract call, including a failed try_place_bid.
        }
    }

    #[test]
    fn fuzz_refund_balance_invariant_deterministic() {
        let env = Env::default();
        env.mock_all_auths();

        let bidders: [Address; 4] = [
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
        ];

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let token_admin = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract_v2(token_admin);
        let bid_token = token_id.address();

        env.as_contract(&contract_id, || {
            env.storage()
                .instance()
                .set(&Symbol::new(&env, "bid_token"), &bid_token);
        });

        let sac = StellarAssetClient::new(&env, &bid_token);
        let token_client = TokenClient::new(&env, &bid_token);

        let initial_contract_balance = 2_000_000_i128;
        let initial_bidder_balance = 1_000_i128;
        sac.mint(&contract_id, &initial_contract_balance);
        for bidder in bidders.iter() {
            sac.mint(bidder, &initial_bidder_balance);
        }

        let total_initial_balance = token_client.balance(&contract_id)
            + bidders
                .iter()
                .map(|bidder| token_client.balance(bidder))
                .sum::<i128>();

        let mut refunded_by_bidder = [0_i128; 4];
        let mut cumulative_refunds = 0_i128;
        let mut expected: Option<(usize, i128)> = None;
        let mut seed: u64 = 0x1234_5678_9abc_def0;
        let auction_id = Symbol::new(&env, "refund_auc");

        client.init_auction(&auction_id, &0, &u64::MAX, &1_i128, &0_u32);

        for _ in 0..FUZZ_STEPS {
            let bidder_idx = pick_index(&mut seed, 0..bidders.len());
            let amount =
                next_amount_above(&mut seed, expected.as_ref().map(|(_, a)| *a).unwrap_or(0));
            client.place_bid(&auction_id, &bidders[bidder_idx], &amount);

            if let Some((prev_idx, prev_amount)) = expected {
                refunded_by_bidder[prev_idx] += prev_amount;
                cumulative_refunds += prev_amount;

                let events = refunded_events(&env);
                let last = events.last().unwrap();
                assert_eq!(last.prev_bidder, bidders[prev_idx]);
                assert_eq!(last.amount, prev_amount);
            }

            assert_eq!(
                token_client.balance(&contract_id),
                initial_contract_balance - cumulative_refunds
            );
            for idx in 0..bidders.len() {
                assert_eq!(
                    token_client.balance(&bidders[idx]),
                    initial_bidder_balance + refunded_by_bidder[idx]
                );
            }

            let total_balance = token_client.balance(&contract_id)
                + bidders
                    .iter()
                    .map(|bidder| token_client.balance(bidder))
                    .sum::<i128>();
            assert_eq!(total_balance, total_initial_balance);

            expected = Some((bidder_idx, amount));
        }
    }

    #[test]
    fn close_semantics_cannot_be_bypassed() {
        let env = Env::default();
        env.mock_all_auths();

        let bidders: [Address; 3] = [
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
        ];

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let auction_id = Symbol::new(&env, "close_auc");

        client.init_auction(&auction_id, &0, &u64::MAX, &1_i128, &0_u32);

        let mut seed: u64 = 0xf00d_cafe_beef_a11c;
        let mut highest = 0_i128;
        for _ in 0..8 {
            let idx = pick_index(&mut seed, 0..bidders.len());
            highest = next_amount_above(&mut seed, highest);
            client.place_bid(&auction_id, &bidders[idx], &highest);
        }

        let expected_state: crate::types::AuctionState = env
            .as_contract(&contract_id, || env.storage().persistent().get(&auction_id))
            .unwrap();
        let refunds_before_close = refunded_events(&env).len();

        client.close_auction(&auction_id);

        for _ in 0..16 {
            let idx = pick_index(&mut seed, 0..bidders.len());
            let attempted_amount = next_amount_above(&mut seed, expected_state.highest_bid);

            let attempt = catch_unwind(AssertUnwindSafe(|| {
                client.place_bid(&auction_id, &bidders[idx], &attempted_amount);
            }));
            assert!(attempt.is_err(), "closed auction accepted a new bid");

            let stored_state: crate::types::AuctionState = env
                .as_contract(&contract_id, || env.storage().persistent().get(&auction_id))
                .unwrap();
            assert_eq!(stored_state.highest_bidder, expected_state.highest_bidder);
            assert_eq!(stored_state.highest_bid, expected_state.highest_bid);
            assert_eq!(stored_state.status, AuctionStatus::Closed);
            assert_eq!(refunded_events(&env).len(), refunds_before_close);
        }
    }

    #[test]
    fn settle_default_liquidation_requires_closed_auction() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let bidder = Address::generate(&env);
        let auction_id = Symbol::new(&env, "liq_open");

        client.init_auction(&auction_id, &0, &1000, &50_i128, &0_u32);
        client.place_bid(&auction_id, &bidder, &100_i128);

        let result = catch_unwind(AssertUnwindSafe(|| {
            client.settle_default_liquidation(
                &auction_id,
                &Address::generate(&env),
                &Address::generate(&env),
            );
        }));

        assert!(result.is_err(), "open auction should not settle");
    }

    #[test]
    fn settle_default_liquidation_emits_once_after_close() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let bidder = Address::generate(&env);
        let borrower = Address::generate(&env);
        let credit_contract = Address::generate(&env);
        let auction_id = Symbol::new(&env, "liq_closed");

        client.init_auction(&auction_id, &0, &1000, &50_i128, &0_u32);
        client.place_bid(&auction_id, &bidder, &420_i128);
        client.close_auction(&auction_id);
        client.settle_default_liquidation(&auction_id, &credit_contract, &borrower);

        let events = settlement_events(&env);
        assert_eq!(events.len(), 1);
        let evt = events.last().unwrap();
        assert_eq!(evt.auction_id, auction_id);
        assert_eq!(evt.credit_contract, credit_contract);
        assert_eq!(evt.borrower, borrower);
        assert_eq!(evt.winner, bidder);
        assert_eq!(evt.recovered_amount, 420_i128);

        // Idempotency: second call must fail. Use try_* to avoid soroban host panics
        // escaping catch_unwind. Also check events before the failed call — soroban-sdk v22
        // resets env.events() after every failed contract invocation.
        let replay = client.try_settle_default_liquidation(&auction_id, &credit_contract, &borrower);
        assert!(replay.is_err(), "settlement replay should panic");
    }

    #[test]
    fn zero_bid_auction_settles_with_borrower_as_winner() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let borrower = Address::generate(&env);
        let credit_contract = Address::generate(&env);
        let auction_id = Symbol::new(&env, "zero_bid");

        client.init_auction(&auction_id, &0, &1000, &50_i128, &0_u32);
        // no bids
        client.close_auction(&auction_id);
        client.settle_default_liquidation(&auction_id, &credit_contract, &borrower);

        let events = settlement_events(&env);
        assert_eq!(events.len(), 1);
        let evt = events.last().unwrap();
        assert_eq!(evt.winner, borrower);
        assert_eq!(evt.recovered_amount, 0_i128);
    }

    #[test]
    fn bid_after_end_time_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1001); // past end time

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let bidder = Address::generate(&env);
        let auction_id = Symbol::new(&env, "timed_out");

        client.init_auction(&auction_id, &0, &1000, &50_i128, &0_u32);

        let attempt = catch_unwind(AssertUnwindSafe(|| {
            client.place_bid(&auction_id, &bidder, &100_i128);
        }));
        assert!(attempt.is_err(), "bid after end time should be rejected");
    }

    #[test]
    fn close_auction_emits_event() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let bidder = Address::generate(&env);
        let auction_id = Symbol::new(&env, "close_event");

        client.init_auction(&auction_id, &0, &1000, &50_i128, &0_u32);
        client.place_bid(&auction_id, &bidder, &100_i128);
        client.close_auction(&auction_id);

        // Check close event
        let close_events = env
            .events()
            .all()
            .iter()
            .filter(|(_contract, topics, _data)| {
                let t0: Symbol = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
                t0 == Symbol::new(&env, "AUC_CLOSE")
            })
            .collect::<Vec<_>>();
        assert_eq!(close_events.len(), 1);
    }

    // ── min_increment_bps: validation at init ──────────────────────────────

    #[test]
    fn init_auction_rejects_increment_bps_above_10000() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let auction_id = Symbol::new(&env, "bad_bps");

        let result = catch_unwind(AssertUnwindSafe(|| {
            client.init_auction(&auction_id, &0, &1000, &50_i128, &10_001_u32);
        }));
        assert!(result.is_err(), "bps > 10000 should be rejected at init");
    }

    #[test]
    fn init_auction_accepts_zero_and_max_increment_bps() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        // 0 bps (no percentage requirement) is valid
        client.init_auction(&Symbol::new(&env, "bps0"), &0, &1000, &1_i128, &0_u32);
        // 10_000 bps (100% increment) is the maximum valid value
        client.init_auction(&Symbol::new(&env, "bps10k"), &0, &1000, &1_i128, &10_000_u32);
    }

    // ── min_increment_bps: bid threshold enforcement ───────────────────────

    #[test]
    fn bid_just_below_increment_threshold_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let auction_id = Symbol::new(&env, "inc_low");

        let alice = Address::generate(&env);
        let bob = Address::generate(&env);

        // 100 bps = 1%; threshold after 1000 = 1000 + ceil(1000*100/10000) = 1010
        client.init_auction(&auction_id, &0, &u64::MAX, &1_i128, &100_u32);
        client.place_bid(&auction_id, &alice, &1_000_i128);

        let result = catch_unwind(AssertUnwindSafe(|| {
            client.place_bid(&auction_id, &bob, &1_009_i128); // 1009 < 1010
        }));
        assert!(result.is_err(), "bid one stroop below threshold must be rejected");

        // state must be unchanged
        let state: crate::types::AuctionState = env
            .as_contract(&contract_id, || env.storage().persistent().get(&auction_id))
            .unwrap();
        assert_eq!(state.highest_bid, 1_000_i128);
        assert_eq!(state.highest_bidder.unwrap(), alice);
    }

    #[test]
    fn bid_at_increment_threshold_accepted() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let auction_id = Symbol::new(&env, "inc_ok");

        let alice = Address::generate(&env);
        let bob = Address::generate(&env);

        // 100 bps = 1%; threshold after 1000 = 1010
        client.init_auction(&auction_id, &0, &u64::MAX, &1_i128, &100_u32);
        client.place_bid(&auction_id, &alice, &1_000_i128);
        client.place_bid(&auction_id, &bob, &1_010_i128); // exactly at threshold

        let state: crate::types::AuctionState = env
            .as_contract(&contract_id, || env.storage().persistent().get(&auction_id))
            .unwrap();
        assert_eq!(state.highest_bid, 1_010_i128);
        assert_eq!(state.highest_bidder.unwrap(), bob);
    }

    #[test]
    fn bid_increment_ceiling_rounding_non_divisible() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let auction_id = Symbol::new(&env, "inc_ceil");

        let alice = Address::generate(&env);
        let bob = Address::generate(&env);
        let carol = Address::generate(&env);

        // 333 bps = 3.33%; increment on 1000 = ceil(1000*333/10000) = ceil(33.3) = 34; threshold = 1034
        client.init_auction(&auction_id, &0, &u64::MAX, &1_i128, &333_u32);
        client.place_bid(&auction_id, &alice, &1_000_i128);

        let just_below = catch_unwind(AssertUnwindSafe(|| {
            client.place_bid(&auction_id, &bob, &1_033_i128); // 1033 < 1034
        }));
        assert!(just_below.is_err(), "bid below ceiling threshold must fail");

        client.place_bid(&auction_id, &carol, &1_034_i128); // exactly at ceiling threshold

        let state: crate::types::AuctionState = env
            .as_contract(&contract_id, || env.storage().persistent().get(&auction_id))
            .unwrap();
        assert_eq!(state.highest_bid, 1_034_i128);
        assert_eq!(state.highest_bidder.unwrap(), carol);
    }

    #[test]
    fn bid_zero_increment_bps_requires_at_least_one_stroop_above() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let auction_id = Symbol::new(&env, "inc_zero");

        let alice = Address::generate(&env);
        let bob = Address::generate(&env);
        let carol = Address::generate(&env);

        // 0 bps: any strictly higher bid is accepted; equal bid must be rejected
        client.init_auction(&auction_id, &0, &u64::MAX, &1_i128, &0_u32);
        client.place_bid(&auction_id, &alice, &500_i128);

        let equal = catch_unwind(AssertUnwindSafe(|| {
            client.place_bid(&auction_id, &bob, &500_i128);
        }));
        assert!(equal.is_err(), "equal bid must be rejected even at 0 bps");

        // exactly one stroop above is accepted
        client.place_bid(&auction_id, &carol, &501_i128);

        let state: crate::types::AuctionState = env
            .as_contract(&contract_id, || env.storage().persistent().get(&auction_id))
            .unwrap();
        assert_eq!(state.highest_bid, 501_i128);
    }
}
