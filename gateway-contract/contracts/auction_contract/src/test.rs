#[cfg(test)]
mod tests {
    use super::super::*;
    use core::convert::TryFrom;
    use core::ops::Range;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::testutils::Events as _;
    use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
    use soroban_sdk::{Address, Env, Symbol, TryFromVal, TryIntoVal};

    const REFUND_TOPIC: &str = "BID_RFDN";
    const SETTLEMENT_TOPIC: &str = "LIQ_SETL";
    const AUCTION_ID: &str = "inv_auc";
    const FUZZ_STEPS: usize = 64;
    const MAX_INCREMENT: u64 = 500;

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
        current + i128::try_from((next_u64(seed) % MAX_INCREMENT) + 1).unwrap()
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

        client.place_bid(&Symbol::new(&env, "auc1"), &alice, &100_i128);
        client.place_bid(&Symbol::new(&env, "auc1"), &bob, &200_i128);

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

        let mut seed: u64 = 0xdeadbeefcafebabe;
        let mut expected: Option<(Address, i128)> = None;
        let mut expected_refunds = 0usize;

        for _ in 0..FUZZ_STEPS {
            let bidder_idx = pick_index(&mut seed, 0..bidders.len());
            let bidder = bidders[bidder_idx].clone();
            let amount =
                next_amount_above(&mut seed, expected.as_ref().map(|(_, a)| *a).unwrap_or(0));

            client.place_bid(&auction_id, &bidder, &amount);

            if let Some((prev_addr, prev_amount)) = expected.clone() {
                let events = refunded_events(&env);
                expected_refunds += 1;
                assert_eq!(events.len(), expected_refunds);
                let evt = events.last().unwrap();
                assert_eq!(evt.prev_bidder, prev_addr);
                assert_eq!(evt.amount, prev_amount);
            }

            expected = Some((bidder.clone(), amount));

            let stored: Option<crate::AuctionState> =
                env.as_contract(&contract_id, || env.storage().persistent().get(&auction_id));
            assert!(stored.is_some(), "stored state must exist");
            let s = stored.unwrap();
            assert_eq!(s.bidder, bidder);
            assert_eq!(s.amount, amount);

            let invalid_bidder_idx = pick_index(&mut seed, 0..bidders.len());
            let invalid_attempt = catch_unwind(AssertUnwindSafe(|| {
                client.place_bid(&auction_id, &bidders[invalid_bidder_idx], &amount);
            }));
            assert!(invalid_attempt.is_err(), "equal bid unexpectedly accepted");

            let stored_after_invalid: crate::AuctionState = env
                .as_contract(&contract_id, || env.storage().persistent().get(&auction_id))
                .unwrap();
            assert_eq!(stored_after_invalid.bidder, bidder);
            assert_eq!(stored_after_invalid.amount, amount);
            assert_eq!(refunded_events(&env).len(), expected_refunds);
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

        let initial_contract_balance = 50_000_i128;
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

        let mut seed: u64 = 0xa11ce_f00d_cafe_beef;
        let mut highest = 0_i128;
        for _ in 0..8 {
            let idx = pick_index(&mut seed, 0..bidders.len());
            highest = next_amount_above(&mut seed, highest);
            client.place_bid(&auction_id, &bidders[idx], &highest);
        }

        let expected_state: crate::AuctionState = env
            .as_contract(&contract_id, || env.storage().persistent().get(&auction_id))
            .unwrap();
        let refunds_before_close = refunded_events(&env).len();

        client.close_auction(&auction_id);

        for _ in 0..16 {
            let idx = pick_index(&mut seed, 0..bidders.len());
            let attempted_amount = next_amount_above(&mut seed, expected_state.amount);

            let attempt = catch_unwind(AssertUnwindSafe(|| {
                client.place_bid(&auction_id, &bidders[idx], &attempted_amount);
            }));
            assert!(attempt.is_err(), "closed auction accepted a new bid");

            let stored_state: crate::AuctionState = env
                .as_contract(&contract_id, || env.storage().persistent().get(&auction_id))
                .unwrap();
            assert_eq!(stored_state, expected_state);
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

        let replay = catch_unwind(AssertUnwindSafe(|| {
            client.settle_default_liquidation(&auction_id, &credit_contract, &borrower);
        }));
        assert!(replay.is_err(), "settlement replay should panic");
        assert_eq!(settlement_events(&env).len(), 1);
    }
}
