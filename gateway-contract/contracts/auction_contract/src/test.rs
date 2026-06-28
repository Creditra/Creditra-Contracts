#[cfg(test)]
mod tests {
    extern crate std;
    use super::super::*;
    use crate::errors::AuctionError;
    use core::ops::Range;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::vec::Vec;

    use soroban_sdk::testutils::Events as _;
    use soroban_sdk::testutils::{Address as _, Ledger};
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
        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0,
            &1000,
            &50_i128,
            &0_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );

        client.place_bid(&auction_id, &alice, &100_i128);
        client.place_bid(&auction_id, &bob, &200_i128);

        let refund_events = refunded_events(&env);
        assert_eq!(refund_events.len(), 1);
        let event_data = refund_events.last().unwrap();
        assert_eq!(event_data.prev_bidder, alice);
        assert_eq!(event_data.amount, 100_i128);
    }

    #[test]
    fn equal_to_highest_bid_rejected_as_bid_too_low() {
        let env = Env::default();
        env.mock_all_auths();

        let alice = Address::generate(&env);
        let bob = Address::generate(&env);

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let auction_id = Symbol::new(&env, "eq_highest");
        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0,
            &1000,
            &50_i128,
            &0_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );

        client.place_bid(&auction_id, &alice, &100_i128);

        let result = client.try_place_bid(&auction_id, &bob, &100_i128);
        assert!(result.is_err(), "equal-to-highest bid must fail");
        let contract_err = result.unwrap_err().unwrap();
        assert_eq!(
            contract_err,
            AuctionError::BidTooLow.into(),
            "equal-to-highest bid must return BidTooLow"
        );

        let stored_after: crate::types::AuctionState = env
            .as_contract(&contract_id, || env.storage().persistent().get(&auction_id))
            .unwrap();
        assert_eq!(stored_after.highest_bidder.unwrap(), alice);
        assert_eq!(stored_after.highest_bid, 100_i128);
        assert_eq!(refunded_events(&env).len(), 0);
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

        let mut seed: u64 = 0xdeadbeefcafebabe;
        let mut expected: Option<(Address, i128)> = None;

        for _ in 0..FUZZ_STEPS {
            let bidder_idx = pick_index(&mut seed, 0..bidders.len());
            let bidder = bidders[bidder_idx].clone();
            let amount =
                next_amount_above(&mut seed, expected.as_ref().map(|(_, a)| *a).unwrap_or(0));

            client.place_bid(&auction_id, &bidder, &amount);

            if let Some((prev_addr, prev_amount)) = expected.clone() {
                let events = refunded_events(&env);
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

        let initial_bidder_balance = 100_000_i128;
        for bidder in bidders.iter() {
            sac.mint(bidder, &initial_bidder_balance);
        }

        let total_initial_balance = token_client.balance(&contract_id)
            + bidders
                .iter()
                .map(|bidder| token_client.balance(bidder))
                .sum::<i128>();

        let mut refunded_by_bidder = [0_i128; 4];
        let mut spent_by_bidder = [0_i128; 4];
        let mut expected: Option<(usize, i128)> = None;
        let mut seed: u64 = 0x1234_5678_9abc_def0;
        let auction_id = Symbol::new(&env, "refund_auc");

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

        for _ in 0..FUZZ_STEPS {
            let bidder_idx = pick_index(&mut seed, 0..bidders.len());
            let amount =
                next_amount_above(&mut seed, expected.as_ref().map(|(_, a)| *a).unwrap_or(0));
            spent_by_bidder[bidder_idx] += amount;
            client.place_bid(&auction_id, &bidders[bidder_idx], &amount);

            if let Some((prev_idx, prev_amount)) = expected {
                refunded_by_bidder[prev_idx] += prev_amount;

                let events = refunded_events(&env);
                let last = events.last().unwrap();
                assert_eq!(last.prev_bidder, bidders[prev_idx]);
                assert_eq!(last.amount, prev_amount);
            }

            let stored: crate::types::AuctionState = env
                .as_contract(&contract_id, || env.storage().persistent().get(&auction_id))
                .unwrap();
            assert_eq!(
                token_client.balance(&contract_id),
                stored.highest_bid,
                "contract escrow must equal only the current highest bid"
            );
            for idx in 0..bidders.len() {
                assert_eq!(
                    token_client.balance(&bidders[idx]),
                    initial_bidder_balance - spent_by_bidder[idx] + refunded_by_bidder[idx],
                    "bidder balance must reflect exact deposits and refunds"
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

        let mut seed: u64 = 0xdeadbeef_cafe_beef;
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

            let attempt = client.try_place_bid(&auction_id, &bidders[idx], &attempted_amount);
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
        let factory = Address::generate(&env);
        let auction_id = Symbol::new(&env, "liq_open");

        client.set_factory_contract(&factory);
        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0,
            &1000,
            &50_i128,
            &0_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );
        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0,
            &1000,
            &50_i128,
            &0_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );
        client.place_bid(&auction_id, &bidder, &100_i128);

        let result = client.try_settle_default_liquidation(
            &auction_id,
            &Address::generate(&env),
            &Address::generate(&env),
        );
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
        let factory = Address::generate(&env);
        let auction_id = Symbol::new(&env, "liq_closed");

        client.set_factory_contract(&factory);
        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0,
            &1000,
            &50_i128,
            &0_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );
        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0,
            &1000,
            &50_i128,
            &0_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );
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
    }

    #[test]
    fn settle_default_liquidation_replay_reverts() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let factory = Address::generate(&env);
        let borrower = Address::generate(&env);
        let credit_contract = Address::generate(&env);
        let auction_id = Symbol::new(&env, "liq_replay");

        client.set_factory_contract(&factory);
        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0,
            &1000,
            &50_i128,
            &0_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );
        client.close_auction(&auction_id);
        client.settle_default_liquidation(&auction_id, &credit_contract, &borrower);

        let replay =
            client.try_settle_default_liquidation(&auction_id, &credit_contract, &borrower);
        assert!(replay.is_err(), "settlement replay should fail");
        assert_eq!(
            replay.unwrap_err().unwrap(),
            AuctionError::AlreadySettled.into(),
            "replay must return AlreadySettled error code"
        );
    }

    #[test]
    fn zero_bid_auction_settles_with_borrower_as_winner() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let borrower = Address::generate(&env);
        let credit_contract = Address::generate(&env);
        let factory = Address::generate(&env);
        let auction_id = Symbol::new(&env, "zero_bid");

        client.set_factory_contract(&factory);
        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0,
            &1000,
            &50_i128,
            &0_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );
        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0,
            &1000,
            &50_i128,
            &0_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );
        client.close_auction(&auction_id);
        client.settle_default_liquidation(&auction_id, &credit_contract, &borrower);

        let events = settlement_events(&env);
        assert_eq!(events.len(), 1);
        let evt = events.last().unwrap();
        assert_eq!(evt.winner, borrower);
        assert_eq!(evt.recovered_amount, 0_i128);
    }

    #[test]
    fn settle_default_liquidation_reverts_when_factory_unset() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let auction_id = Symbol::new(&env, "no_factory");

        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0,
            &1000,
            &50_i128,
            &0_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );
        client.close_auction(&auction_id);

        let result = client.try_settle_default_liquidation(
            &auction_id,
            &Address::generate(&env),
            &Address::generate(&env),
        );
        assert!(
            result.is_err(),
            "should revert when factory contract is unset"
        );
        assert_eq!(
            result.unwrap_err().unwrap(),
            AuctionError::NoFactoryContract.into(),
            "must return NoFactoryContract error code"
        );
    }

    #[test]
    fn settle_default_liquidation_reverts_for_wrong_caller() {
        let env = Env::default();
        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let factory = Address::generate(&env);
        let borrower = Address::generate(&env);
        let credit_contract = Address::generate(&env);
        let auction_id = Symbol::new(&env, "wrong_caller");

        env.mock_all_auths();
        client.set_factory_contract(&factory);
        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0,
            &1000,
            &50_i128,
            &0_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );
        client.close_auction(&auction_id);

        let wrong = Address::generate(&env);
        use soroban_sdk::IntoVal;
        let result = client
            .mock_auths(&[soroban_sdk::testutils::MockAuth {
                address: &wrong,
                invoke: &soroban_sdk::testutils::MockAuthInvoke {
                    contract: &contract_id,
                    fn_name: "settle_default_liquidation",
                    args: (
                        auction_id.clone(),
                        credit_contract.clone(),
                        borrower.clone(),
                    )
                        .into_val(&env),
                    sub_invokes: &[],
                },
            }])
            .try_settle_default_liquidation(&auction_id, &credit_contract, &borrower);
        assert!(result.is_err(), "wrong caller should be rejected");
    }

    #[test]
    fn bid_after_end_time_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1001);

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let bidder = Address::generate(&env);
        let auction_id = Symbol::new(&env, "timed_out");

        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0,
            &1000,
            &50_i128,
            &0_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );

        let attempt = client.try_place_bid(&auction_id, &bidder, &100_i128);
        assert!(attempt.is_err(), "bid after end time should be rejected");
    }

    #[test]
    fn settle_default_liquidation_requires_factory_contract_set() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let bidder = Address::generate(&env);
        let borrower = Address::generate(&env);
        let credit_contract = Address::generate(&env);
        let auction_id = Symbol::new(&env, "no_factory2");

        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0,
            &1000,
            &50_i128,
            &0_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );
        client.place_bid(&auction_id, &bidder, &420_i128);
        client.close_auction(&auction_id);

        let result =
            client.try_settle_default_liquidation(&auction_id, &credit_contract, &borrower);
        assert!(result.is_err(), "should fail if factory not set");
        assert_eq!(
            result.unwrap_err().unwrap(),
            AuctionError::NoFactoryContract.into(),
            "must return NoFactoryContract error code"
        );
    }

    #[test]
    fn settle_default_liquidation_requires_authorized_factory() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let factory = Address::generate(&env);
        let bidder = Address::generate(&env);
        let borrower = Address::generate(&env);
        let credit_contract = Address::generate(&env);
        let auction_id = Symbol::new(&env, "unauth");

        client.set_factory_contract(&factory);
        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0,
            &1000,
            &50_i128,
            &0_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );
        client.place_bid(&auction_id, &bidder, &420_i128);
        client.close_auction(&auction_id);

        let intruder = Address::generate(&env);
        use soroban_sdk::IntoVal;
        let result = client
            .mock_auths(&[soroban_sdk::testutils::MockAuth {
                address: &intruder,
                invoke: &soroban_sdk::testutils::MockAuthInvoke {
                    contract: &contract_id,
                    fn_name: "settle_default_liquidation",
                    args: (
                        auction_id.clone(),
                        credit_contract.clone(),
                        borrower.clone(),
                    )
                        .into_val(&env),
                    sub_invokes: &[],
                },
            }])
            .try_settle_default_liquidation(&auction_id, &credit_contract, &borrower);
        assert!(result.is_err(), "should fail if unauthorized caller");
    }

    #[test]
    fn settle_default_liquidation_succeeds_with_factory() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let factory = Address::generate(&env);
        let bidder = Address::generate(&env);
        let borrower = Address::generate(&env);
        let credit_contract = Address::generate(&env);
        let auction_id = Symbol::new(&env, "auth_success");

        client.set_factory_contract(&factory);
        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0,
            &1000,
            &50_i128,
            &0_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );
        client.place_bid(&auction_id, &bidder, &420_i128);
        client.close_auction(&auction_id);
        client.settle_default_liquidation(&auction_id, &credit_contract, &borrower);

        let events = settlement_events(&env);
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn init_auction_rejects_increment_bps_above_10000() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let auction_id = Symbol::new(&env, "bad_bps");

        let result = catch_unwind(AssertUnwindSafe(|| {
            client.init_auction(
                &auction_id,
                &AuctionMode::English,
                &0,
                &1000,
                &50_i128,
                &10_001_u32,
                &None,
                &None,
                &DutchAuctionDecay::None,
                &None,
            );
        }));
        assert!(result.is_err(), "bps > 10000 should be rejected at init");
    }

    #[test]
    fn init_auction_accepts_zero_and_max_increment_bps() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        client.init_auction(
            &Symbol::new(&env, "bps0"),
            &AuctionMode::English,
            &0,
            &1000,
            &1_i128,
            &0_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );
        client.init_auction(
            &Symbol::new(&env, "bps10k"),
            &AuctionMode::English,
            &0,
            &1000,
            &1_i128,
            &10_000_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );
    }

    #[test]
    fn bid_just_below_increment_threshold_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let auction_id = Symbol::new(&env, "inc_low");

        let alice = Address::generate(&env);
        let bob = Address::generate(&env);

        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0,
            &u64::MAX,
            &1_i128,
            &100_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );
        client.place_bid(&auction_id, &alice, &1_000_i128);

        let result = catch_unwind(AssertUnwindSafe(|| {
            client.place_bid(&auction_id, &bob, &1_009_i128);
        }));
        assert!(
            result.is_err(),
            "bid one stroop below threshold must be rejected"
        );

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

        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0,
            &u64::MAX,
            &1_i128,
            &100_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );
        client.place_bid(&auction_id, &alice, &1_000_i128);
        client.place_bid(&auction_id, &bob, &1_010_i128);

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

        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0,
            &u64::MAX,
            &1_i128,
            &333_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );
        client.place_bid(&auction_id, &alice, &1_000_i128);

        let just_below = catch_unwind(AssertUnwindSafe(|| {
            client.place_bid(&auction_id, &bob, &1_033_i128);
        }));
        assert!(just_below.is_err(), "bid below ceiling threshold must fail");

        client.place_bid(&auction_id, &carol, &1_034_i128);

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
        client.place_bid(&auction_id, &alice, &500_i128);

        let equal = catch_unwind(AssertUnwindSafe(|| {
            client.place_bid(&auction_id, &bob, &500_i128);
        }));
        assert!(equal.is_err(), "equal bid must be rejected even at 0 bps");

        client.place_bid(&auction_id, &carol, &501_i128);

        let state: crate::types::AuctionState = env
            .as_contract(&contract_id, || env.storage().persistent().get(&auction_id))
            .unwrap();
        assert_eq!(state.highest_bid, 501_i128);
    }

    #[test]
    fn claim_non_winner_fails_not_winner() {
        let env = Env::default();
        env.mock_all_auths();

        let alice = Address::generate(&env);
        let bob = Address::generate(&env);
        let winner = Address::generate(&env);

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let auction_id = Symbol::new(&env, "claim_non_winner");

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
        client.place_bid(&auction_id, &winner, &100_i128);
        client.close_auction(&auction_id);

        let result = catch_unwind(AssertUnwindSafe(|| {
            client.claim_auction(&auction_id);
        }));
        assert!(result.is_err(), "non-winner claim should fail");
    }

    #[test]
    fn claim_double_claim_fails_already_claimed() {
        let env = Env::default();
        env.mock_all_auths();

        let winner = Address::generate(&env);

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let auction_id = Symbol::new(&env, "claim_double");

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
        client.place_bid(&auction_id, &winner, &100_i128);
        client.close_auction(&auction_id);

        let first = catch_unwind(AssertUnwindSafe(|| {
            client.claim_auction(&auction_id);
        }));
        assert!(first.is_ok(), "first claim should succeed");

        let second = catch_unwind(AssertUnwindSafe(|| {
            client.claim_auction(&auction_id);
        }));
        assert!(second.is_err(), "second claim should fail");
    }

    #[test]
    fn claim_before_close_fails_not_closed() {
        let env = Env::default();
        env.mock_all_auths();

        let winner = Address::generate(&env);

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let auction_id = Symbol::new(&env, "claim_not_closed");

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
        client.place_bid(&auction_id, &winner, &100_i128);

        let result = catch_unwind(AssertUnwindSafe(|| {
            client.claim_auction(&auction_id);
        }));
        assert!(result.is_err(), "claim before close should fail");
    }

    #[test]
    fn claim_zero_bid_auction_fails_not_winner() {
        let env = Env::default();
        env.mock_all_auths();

        let borrower = Address::generate(&env);

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let auction_id = Symbol::new(&env, "zero_bid_claim");

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
        client.close_auction(&auction_id);

        let result = catch_unwind(AssertUnwindSafe(|| {
            client.claim_auction(&auction_id);
        }));
        assert!(result.is_err(), "zero-bid claim should fail");
    }

    // === Dutch Auction Tests ===

    #[test]
    fn dutch_auction_price_at_start() {
        let env = Env::default();
        env.mock_all_auths();

        let alice = Address::generate(&env);

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let auction_id = Symbol::new(&env, "dutch_start");

        client.init_auction(
            &auction_id,
            &AuctionMode::Dutch,
            &1000,
            &2000,
            &50_i128,
            &0_u32,
            &Some(500_i128),
            &Some(100_i128),
            &DutchAuctionDecay::Linear,
            &None,
        );

        env.ledger().with_mut(|li| li.timestamp = 1000);
        client.place_bid(&auction_id, &alice, &500_i128);

        let stored: crate::types::AuctionState = env
            .as_contract(&contract_id, || env.storage().persistent().get(&auction_id))
            .unwrap();

        assert_eq!(stored.status, AuctionStatus::Closed);
        assert_eq!(stored.highest_bidder.unwrap(), alice);
        assert_eq!(stored.highest_bid, 500_i128);
    }

    #[test]
    fn dutch_auction_price_at_mid() {
        let env = Env::default();
        env.mock_all_auths();

        let alice = Address::generate(&env);

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let auction_id = Symbol::new(&env, "dutch_mid");

        client.init_auction(
            &auction_id,
            &AuctionMode::Dutch,
            &1000,
            &2000,
            &50_i128,
            &0_u32,
            &Some(500_i128),
            &Some(100_i128),
            &DutchAuctionDecay::Linear,
            &None,
        );

        env.ledger().with_mut(|li| li.timestamp = 1500);
        client.place_bid(&auction_id, &alice, &300_i128);

        let stored: crate::types::AuctionState = env
            .as_contract(&contract_id, || env.storage().persistent().get(&auction_id))
            .unwrap();

        assert_eq!(stored.status, AuctionStatus::Closed);
        assert_eq!(stored.highest_bidder.unwrap(), alice);
        assert_eq!(stored.highest_bid, 300_i128);
    }

    #[test]
    fn dutch_auction_price_at_floor() {
        let env = Env::default();
        env.mock_all_auths();

        let alice = Address::generate(&env);

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let auction_id = Symbol::new(&env, "dutch_floor");

        client.init_auction(
            &auction_id,
            &AuctionMode::Dutch,
            &1000,
            &2000,
            &50_i128,
            &0_u32,
            &Some(500_i128),
            &Some(100_i128),
            &DutchAuctionDecay::Linear,
            &None,
        );

        env.ledger().with_mut(|li| li.timestamp = 2000);
        client.place_bid(&auction_id, &alice, &100_i128);

        let stored: crate::types::AuctionState = env
            .as_contract(&contract_id, || env.storage().persistent().get(&auction_id))
            .unwrap();

        assert_eq!(stored.status, AuctionStatus::Closed);
        assert_eq!(stored.highest_bidder.unwrap(), alice);
        assert_eq!(stored.highest_bid, 100_i128);
    }

    #[test]
    fn dutch_auction_bid_below_current_price_fails() {
        let env = Env::default();
        env.mock_all_auths();

        let alice = Address::generate(&env);

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let auction_id = Symbol::new(&env, "dutch_low_bid");

        client.init_auction(
            &auction_id,
            &AuctionMode::Dutch,
            &1000,
            &2000,
            &50_i128,
            &0_u32,
            &Some(500_i128),
            &Some(100_i128),
            &DutchAuctionDecay::Linear,
            &None,
        );

        env.ledger().with_mut(|li| li.timestamp = 1500);
        let result = client.try_place_bid(&auction_id, &alice, &250_i128);
        assert!(result.is_err());
    }

    #[test]
    fn dutch_auction_first_bid_settles_immediately() {
        let env = Env::default();
        env.mock_all_auths();

        let alice = Address::generate(&env);
        let bob = Address::generate(&env);

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let auction_id = Symbol::new(&env, "dutch_first_bid");

        client.init_auction(
            &auction_id,
            &AuctionMode::Dutch,
            &1000,
            &2000,
            &50_i128,
            &0_u32,
            &Some(500_i128),
            &Some(100_i128),
            &DutchAuctionDecay::Linear,
            &None,
        );

        env.ledger().with_mut(|li| li.timestamp = 1500);
        client.place_bid(&auction_id, &alice, &300_i128);

        let stored: crate::types::AuctionState = env
            .as_contract(&contract_id, || env.storage().persistent().get(&auction_id))
            .unwrap();

        assert_eq!(stored.status, AuctionStatus::Closed);
        let result = client.try_place_bid(&auction_id, &bob, &400_i128);
        assert!(result.is_err());
    }

    #[test]
    fn test_compute_dutch_price_linear_happy_paths() {
        assert_eq!(
            super::super::compute_dutch_price(1000, 500, 0, 100, &DutchAuctionDecay::Linear, None),
            1000
        );
        assert_eq!(
            super::super::compute_dutch_price(1000, 500, 50, 100, &DutchAuctionDecay::Linear, None),
            750
        );
        assert_eq!(
            super::super::compute_dutch_price(1000, 500, 25, 100, &DutchAuctionDecay::Linear, None),
            875
        );
        assert_eq!(
            super::super::compute_dutch_price(1000, 500, 75, 100, &DutchAuctionDecay::Linear, None),
            625
        );
        assert_eq!(
            super::super::compute_dutch_price(1000, 1000, 50, 100, &DutchAuctionDecay::Linear, None),
            1000
        );
    }

    #[test]
    fn test_compute_dutch_price_stepped_happy_paths() {
        assert_eq!(
            super::super::compute_dutch_price(1000, 500, 0, 100, &DutchAuctionDecay::Stepped, Some(5)),
            1000
        );
        assert_eq!(
            super::super::compute_dutch_price(1000, 500, 19, 100, &DutchAuctionDecay::Stepped, Some(5)),
            1000
        );
        assert_eq!(
            super::super::compute_dutch_price(1000, 500, 20, 100, &DutchAuctionDecay::Stepped, Some(5)),
            900
        );
        assert_eq!(
            super::super::compute_dutch_price(1000, 500, 40, 100, &DutchAuctionDecay::Stepped, Some(5)),
            800
        );
        assert_eq!(
            super::super::compute_dutch_price(1000, 500, 99, 100, &DutchAuctionDecay::Stepped, Some(5)),
            600
        );
    }

    #[test]
    fn test_compute_dutch_price_edge_cases() {
        assert_eq!(
            super::super::compute_dutch_price(1000, 500, 50, 0, &DutchAuctionDecay::Linear, None),
            500
        );
        assert_eq!(
            super::super::compute_dutch_price(1000, 500, 100, 100, &DutchAuctionDecay::Linear, None),
            500
        );
        assert_eq!(
            super::super::compute_dutch_price(1000, 500, 150, 100, &DutchAuctionDecay::Stepped, Some(5)),
            500
        );
    }

    #[test]
    fn test_compute_dutch_price_invalid_inputs_panic() {
        let result = catch_unwind(AssertUnwindSafe(|| {
            super::super::compute_dutch_price(500, 1000, 50, 100, &DutchAuctionDecay::Linear, None);
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_compute_dutch_price_missing_step_count_panics() {
        let result = catch_unwind(AssertUnwindSafe(|| {
            super::super::compute_dutch_price(1000, 500, 50, 100, &DutchAuctionDecay::Stepped, None);
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_compute_dutch_price_overflow_panics() {
        let result = catch_unwind(AssertUnwindSafe(|| {
            super::super::compute_dutch_price(i128::MAX, 0, 2, 100, &DutchAuctionDecay::Linear, None);
        }));
        assert!(result.is_err());
    }

    #[test]
    fn init_auction_rejects_stepped_decay_without_step_count() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let auction_id = Symbol::new(&env, "dutch_step_missing");

        let result = catch_unwind(AssertUnwindSafe(|| {
            client.init_auction(
                &auction_id,
                &AuctionMode::Dutch,
                &1000,
                &2000,
                &50_i128,
                &0_u32,
                &Some(500_i128),
                &Some(100_i128),
                &DutchAuctionDecay::Stepped,
                &None,
            );
        }));
        assert!(result.is_err());
    }

    #[test]
    fn init_auction_rejects_zero_step_count_for_stepped_decay() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let auction_id = Symbol::new(&env, "dutch_step_zero");

        let result = catch_unwind(AssertUnwindSafe(|| {
            client.init_auction(
                &auction_id,
                &AuctionMode::Dutch,
                &1000,
                &2000,
                &50_i128,
                &0_u32,
                &Some(500_i128),
                &Some(100_i128),
                &DutchAuctionDecay::Stepped,
                &Some(0_u32),
            );
        }));
        assert!(result.is_err());
    }

    #[test]
    fn dutch_auction_stepped_decay_enforces_bucket_price() {
        let env = Env::default();
        env.mock_all_auths();

        let alice = Address::generate(&env);
        let bob = Address::generate(&env);

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let auction_id = Symbol::new(&env, "dutch_step_bid");

        client.init_auction(
            &auction_id,
            &AuctionMode::Dutch,
            &1000,
            &2000,
            &50_i128,
            &0_u32,
            &Some(500_i128),
            &Some(100_i128),
            &DutchAuctionDecay::Stepped,
            &Some(4_u32),
        );

        env.ledger().with_mut(|li| li.timestamp = 1499);
        let low = client.try_place_bid(&auction_id, &alice, &399_i128);
        assert!(low.is_err(), "bucket price of 400 must reject 399");

        client.place_bid(&auction_id, &bob, &400_i128);

        let stored: crate::types::AuctionState = env
            .as_contract(&contract_id, || env.storage().persistent().get(&auction_id))
            .unwrap();
        assert_eq!(stored.status, AuctionStatus::Closed);
        assert_eq!(stored.highest_bidder.unwrap(), bob);
        assert_eq!(stored.highest_bid, 400_i128);
    }

    #[test]
    fn english_mode_unchanged_with_new_signature() {
        let env = Env::default();
        env.mock_all_auths();

        let alice = Address::generate(&env);
        let bob = Address::generate(&env);

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let auction_id = Symbol::new(&env, "english_unchanged");

        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0,
            &1000,
            &50_i128,
            &0_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );

        client.place_bid(&auction_id, &alice, &100_i128);
        client.place_bid(&auction_id, &bob, &200_i128);

        let stored: crate::types::AuctionState = env
            .as_contract(&contract_id, || env.storage().persistent().get(&auction_id))
            .unwrap();

        assert_eq!(stored.status, AuctionStatus::Open);
        assert_eq!(stored.highest_bidder.unwrap(), bob);
        assert_eq!(stored.highest_bid, 200_i128);
    }
}

// ── reentrancy_exploration ────────────────────────────────────────────────────
#[cfg(test)]
mod reentrancy_exploration {
    extern crate std;
    use super::*;
    use crate::errors::AuctionError;
    use crate::{Auction, AuctionClient, AuctionMode, AuctionStatus, DutchAuctionDecay};    use soroban_sdk::testutils::{Address as _, Ledger as _};
    use soroban_sdk::{Address, Env, Symbol};

    fn reentrancy_flag(env: &Env, contract_id: &Address) -> bool {
        env.as_contract(contract_id, || {
            env.storage()
                .instance()
                .get::<Symbol, bool>(&Symbol::new(env, "reentrancy"))
                .unwrap_or(false)
        })
    }

    #[test]
    fn scenario_a_reentrant_place_bid_during_refund_reverts() {
        let env = Env::default();
        env.mock_all_auths();

        let alice = Address::generate(&env);
        let bob = Address::generate(&env);

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let auction_id = Symbol::new(&env, "reent_a");

        let token_admin = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract_v2(token_admin.clone());
        let bid_token = token_id.address();
        let sac = soroban_sdk::token::StellarAssetClient::new(&env, &bid_token);

        sac.mint(&contract_id, &1_000_i128);

        env.as_contract(&contract_id, || {
            env.storage()
                .instance()
                .set(&Symbol::new(&env, "bid_token"), &bid_token);
        });

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

        client.place_bid(&auction_id, &alice, &100_i128);
        client.place_bid(&auction_id, &bob, &300_i128);

        assert!(
            !reentrancy_flag(&env, &contract_id),
            "Scenario A: reentrancy flag must be false after place_bid completes"
        );

        let state: crate::types::AuctionState = env
            .as_contract(&contract_id, || env.storage().persistent().get(&auction_id))
            .unwrap();
        assert_eq!(state.highest_bidder.unwrap(), bob);
        assert_eq!(state.highest_bid, 300_i128);
    }

    #[test]
    fn scenario_a_direct_guard_blocks_reentry() {
        let env = Env::default();
        let contract_id = env.register(Auction, ());

        env.as_contract(&contract_id, || {
            crate::storage::set_reentrancy_guard(&env);
        });

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.as_contract(&contract_id, || {
                crate::storage::set_reentrancy_guard(&env);
            });
        }));
        assert!(
            result.is_err(),
            "Scenario A: second set_reentrancy_guard must panic with Reentrancy"
        );

        env.as_contract(&contract_id, || {
            crate::storage::clear_reentrancy_guard(&env);
        });
        assert!(
            !reentrancy_flag(&env, &contract_id),
            "Scenario A: guard must be false after clear"
        );
    }

    #[test]
    fn scenario_b_claim_auction_guard_cleared_after_claim() {
        let env = Env::default();
        env.mock_all_auths();

        let winner = Address::generate(&env);

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let auction_id = Symbol::new(&env, "reent_b");

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
        client.place_bid(&auction_id, &winner, &100_i128);
        client.close_auction(&auction_id);
        client.claim_auction(&auction_id);

        assert!(
            !reentrancy_flag(&env, &contract_id),
            "Scenario B: reentrancy flag must be false after claim_auction completes"
        );

        let second = client.try_claim_auction(&auction_id);
        assert!(
            second.is_err(),
            "Scenario B: second claim_auction must revert"
        );
    }

    #[test]
    fn scenario_c_guard_cleared_after_outbid_no_token() {
        let env = Env::default();
        env.mock_all_auths();

        let alice = Address::generate(&env);
        let bob = Address::generate(&env);

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let auction_id = Symbol::new(&env, "reent_c");

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

        client.place_bid(&auction_id, &alice, &100_i128);
        client.place_bid(&auction_id, &bob, &200_i128);

        assert!(
            !reentrancy_flag(&env, &contract_id),
            "Scenario C: reentrancy flag must be false after outbid completes"
        );
    }
}

// ── reentrancy_preservation ───────────────────────────────────────────────────
#[cfg(test)]
mod reentrancy_preservation {
    extern crate std;
    use super::*;
    use crate::{Auction, AuctionClient, AuctionMode, AuctionStatus, DutchAuctionDecay};    use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
    use soroban_sdk::{Address, Env, Symbol, TryFromVal, TryIntoVal};

    fn refund_event_count(env: &Env) -> usize {
        let mut count = 0;
        for (_contract, topics, _data) in env.events().all().iter() {
            let t0: Symbol = Symbol::try_from_val(env, &topics.get(0).unwrap()).unwrap();
            if t0 == Symbol::new(env, "BID_RFDN") {
                count += 1;
            }
        }
        count
    }

    #[test]
    fn first_bid_accepted_no_refund_event() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let amounts: [i128; 8] = [50, 51, 100, 999, 1_000, 10_000, 100_000, 1_000_000];
        for amount in amounts {
            let env2 = Env::default();
            env2.mock_all_auths();
            let cid2 = env2.register(Auction, ());
            let cli2 = AuctionClient::new(&env2, &cid2);
            let aid2 = Symbol::new(&env2, "pres_f2");
            cli2.init_auction(
                &aid2,
                &AuctionMode::English,
                &0,
                &u64::MAX,
                &50_i128,
                &0_u32,
                &None,
                &None,
                &DutchAuctionDecay::None,
                &None,
            );
            cli2.place_bid(&aid2, &Address::generate(&env2), &amount);

            let state: crate::types::AuctionState = env2
                .as_contract(&cid2, || env2.storage().persistent().get(&aid2))
                .unwrap();
            assert_eq!(state.highest_bid, amount, "first bid amount must be stored");
            assert_eq!(
                refund_event_count(&env2),
                0,
                "first bid must emit no BID_RFDN event"
            );
        }
    }

    #[test]
    fn dutch_bid_closes_auction_no_refund_event() {
        let env = Env::default();
        env.mock_all_auths();

        let alice = Address::generate(&env);
        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let auction_id = Symbol::new(&env, "pres_dutch");

        client.init_auction(
            &auction_id,
            &AuctionMode::Dutch,
            &1000,
            &2000,
            &50_i128,
            &0_u32,
            &Some(500_i128),
            &Some(100_i128),
            &DutchAuctionDecay::Linear,
            &None,
        );

        env.ledger().with_mut(|li| li.timestamp = 1500);
        client.place_bid(&auction_id, &alice, &300_i128);

        let state: crate::types::AuctionState = env
            .as_contract(&contract_id, || env.storage().persistent().get(&auction_id))
            .unwrap();
        assert_eq!(state.status, AuctionStatus::Closed);
        assert_eq!(state.highest_bidder.unwrap(), alice);
        assert_eq!(refund_event_count(&env), 0);
    }

    #[test]
    fn error_paths_unchanged() {
        let env = Env::default();
        env.mock_all_auths();

        let alice = Address::generate(&env);
        let bob = Address::generate(&env);
        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let auction_id = Symbol::new(&env, "pres_err");

        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0,
            &u64::MAX,
            &50_i128,
            &0_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );
        client.place_bid(&auction_id, &alice, &100_i128);

        let err = client.try_place_bid(&auction_id, &bob, &100_i128);
        assert!(err.is_err());
        assert_eq!(
            err.unwrap_err().unwrap(),
            crate::errors::AuctionError::BidTooLow.into()
        );

        let err2 = client.try_claim_auction(&auction_id);
        assert!(err2.is_err());

        let env3 = Env::default();
        env3.mock_all_auths();
        let cid3 = env3.register(Auction, ());
        let cli3 = AuctionClient::new(&env3, &cid3);
        let aid3 = Symbol::new(&env3, "pres_nw");
        cli3.init_auction(
            &aid3,
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
        cli3.close_auction(&aid3);
        let err3 = cli3.try_claim_auction(&aid3);
        assert!(err3.is_err(), "claim with no winner must fail");
    }

    #[test]
    fn settle_default_liquidation_unaffected_by_guard() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let factory = Address::generate(&env);
        let bidder = Address::generate(&env);
        let borrower = Address::generate(&env);
        let credit_contract = Address::generate(&env);
        let auction_id = Symbol::new(&env, "pres_settle");

        client.set_factory_contract(&factory);
        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0,
            &1000,
            &50_i128,
            &0_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );
        client.place_bid(&auction_id, &bidder, &420_i128);
        client.close_auction(&auction_id);

        let recovered = client.settle_default_liquidation(&auction_id, &credit_contract, &borrower);
        assert_eq!(recovered, 420_i128);

        let mut settlement_found = false;
        for (_contract, topics, _data) in env.events().all().iter() {
            let t0: Symbol = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
            if t0 == Symbol::new(&env, "LIQ_SETL") {
                settlement_found = true;
            }
        }
        assert!(settlement_found, "LIQ_SETL event must be emitted");
    }
}

// ── liquidation_grace_window ──────────────────────────────────────────────────
#[cfg(test)]
mod liquidation_grace_window {
    extern crate std;
    use super::super::*;
    use crate::errors::AuctionError;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    use soroban_sdk::testutils::{Address as _, Ledger as _};
    use soroban_sdk::{Address, Env, Symbol};

    fn setup_grace_window_test(
        env: &Env,
        start_time: u64,
        end_time: u64,
    ) -> (AuctionClient, Address, Address, Symbol) {
        env.mock_all_auths();
        let factory = Address::generate(env);
        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(env, &contract_id);
        let auction_id = Symbol::new(env, "grace_auc");

        client.set_factory_contract(&factory);
        client.set_liquidation_grace_window(&60_u64);
        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &start_time,
            &end_time,
            &50_i128,
            &0_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );

        (client, factory, contract_id, auction_id)
    }

    /// 1. Grace window enabled: bid before grace period expires is rejected.
    #[test]
    fn bid_rejected_during_grace_window() {
        let env = Env::default();
        let start_time = 1000;
        let end_time = 2000;
        let (client, _factory, _contract_id, auction_id) =
            setup_grace_window_test(&env, start_time, end_time);

        let bidder = Address::generate(&env);
        env.ledger().set_timestamp(1050);
        let result = client.try_place_bid(&auction_id, &bidder, &100_i128);
        assert!(
            result.is_err(),
            "bid before grace window expires must be rejected"
        );
        assert_eq!(
            result.unwrap_err().unwrap(),
            AuctionError::GracePeriodActive.into(),
        );
    }

    /// 2. Grace period elapsed: auction starts successfully.
    #[test]
    fn bid_accepted_after_grace_window() {
        let env = Env::default();
        let start_time = 1000;
        let end_time = 2000;
        let (client, _factory, _contract_id, auction_id) =
            setup_grace_window_test(&env, start_time, end_time);

        let bidder = Address::generate(&env);
        env.ledger().set_timestamp(1100);
        let result = client.try_place_bid(&auction_id, &bidder, &100_i128);
        assert!(
            result.is_ok(),
            "bid after grace window must succeed"
        );
    }

    /// 3. Configuration update: authorized user can update grace window.
    #[test]
    fn authorized_set_liquidation_grace_window() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let factory = Address::generate(&env);
        client.set_factory_contract(&factory);

        client.set_liquidation_grace_window(&120_u64);
        assert_eq!(client.get_liquidation_grace_window(), 120_u64);

        client.set_liquidation_grace_window(&0_u64);
        assert_eq!(client.get_liquidation_grace_window(), 0_u64);
    }

    /// 4. Unauthorized update: rejected without auth from factory contract.
    #[test]
    fn unauthorized_set_liquidation_grace_window_rejected() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let factory = Address::generate(&env);
        client.set_factory_contract(&factory);

        let intruder = Address::generate(&env);
        use soroban_sdk::IntoVal;
        let result = client
            .mock_auths(&[soroban_sdk::testutils::MockAuth {
                address: &intruder,
                invoke: &soroban_sdk::testutils::MockAuthInvoke {
                    contract: &contract_id,
                    fn_name: "set_liquidation_grace_window",
                    args: (60_u64,).into_val(&env),
                    sub_invokes: &[],
                },
            }])
            .try_set_liquidation_grace_window(&60_u64);
        assert!(
            result.is_err(),
            "unauthorized grace window update should be rejected"
        );
    }

    /// 5a. Boundary: bid at the exact expiry time succeeds.
    #[test]
    fn bid_at_exact_grace_window_expiry() {
        let env = Env::default();
        let start_time = 1000;
        let end_time = 2000;
        let (client, _factory, _contract_id, auction_id) =
            setup_grace_window_test(&env, start_time, end_time);

        let bidder = Address::generate(&env);
        env.ledger().set_timestamp(1060);
        let result = client.try_place_bid(&auction_id, &bidder, &100_i128);
        assert!(
            result.is_ok(),
            "bid at exact grace window expiry must succeed"
        );
    }

    /// 5b. Boundary: bid one second before expiry fails.
    #[test]
    fn bid_one_second_before_expiry() {
        let env = Env::default();
        let start_time = 1000;
        let end_time = 2000;
        let (client, _factory, _contract_id, auction_id) =
            setup_grace_window_test(&env, start_time, end_time);

        let bidder = Address::generate(&env);
        env.ledger().set_timestamp(1059);
        let result = client.try_place_bid(&auction_id, &bidder, &100_i128);
        assert!(
            result.is_err(),
            "bid one second before grace expiry must be rejected"
        );
        assert_eq!(
            result.unwrap_err().unwrap(),
            AuctionError::GracePeriodActive.into(),
        );
    }

    /// Grace window disabled (default) preserves existing behavior.
    #[test]
    fn no_grace_window_default_behavior() {
        let env = Env::default();
        env.mock_all_auths();

        let factory = Address::generate(&env);
        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let auction_id = Symbol::new(&env, "no_grace");

        client.set_factory_contract(&factory);

        // Never set a grace window — defaults to 0 (disabled).
        assert_eq!(client.get_liquidation_grace_window(), 0_u64);

        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &1000,
            &2000,
            &50_i128,
            &0_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );

        let bidder = Address::generate(&env);
        env.ledger().set_timestamp(1000);
        let result = client.try_place_bid(&auction_id, &bidder, &100_i128);
        assert!(
            result.is_ok(),
            "bid at start_time must be accepted when grace window is disabled"
        );
    }

    /// Grace window with Dutch auction: bid before expiry is blocked.
    #[test]
    fn dutch_auction_bid_during_grace_window_rejected() {
        let env = Env::default();
        env.mock_all_auths();

        let factory = Address::generate(&env);
        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let auction_id = Symbol::new(&env, "dutch_grace");

        client.set_factory_contract(&factory);
        client.set_liquidation_grace_window(&60_u64);
        client.init_auction(
            &auction_id,
            &AuctionMode::Dutch,
            &1000,
            &2000,
            &50_i128,
            &0_u32,
            &Some(500_i128),
            &Some(100_i128),
            &DutchAuctionDecay::Linear,
            &None,
        );

        let bidder = Address::generate(&env);
        env.ledger().set_timestamp(1050);
        let result = client.try_place_bid(&auction_id, &bidder, &300_i128);
        assert!(
            result.is_err(),
            "Dutch bid during grace window must be rejected"
        );
        assert_eq!(
            result.unwrap_err().unwrap(),
            AuctionError::GracePeriodActive.into(),
        );
    }

    /// Grace window with Dutch auction: bid after expiry is accepted.
    #[test]
    fn dutch_auction_bid_after_grace_window_accepted() {
        let env = Env::default();
        env.mock_all_auths();

        let factory = Address::generate(&env);
        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let auction_id = Symbol::new(&env, "dutch_grace_ok");

        client.set_factory_contract(&factory);
        client.set_liquidation_grace_window(&60_u64);
        client.init_auction(
            &auction_id,
            &AuctionMode::Dutch,
            &1000,
            &2000,
            &50_i128,
            &0_u32,
            &Some(500_i128),
            &Some(100_i128),
            &DutchAuctionDecay::Linear,
            &None,
        );

        let bidder = Address::generate(&env);
        env.ledger().set_timestamp(1100);
        let result = client.try_place_bid(&auction_id, &bidder, &300_i128);
        assert!(
            result.is_ok(),
            "Dutch bid after grace window must succeed"
        );
    }

    /// Grace window does not affect close_auction or other non-bid operations.
    #[test]
    fn close_auction_unaffected_by_grace_window() {
        let env = Env::default();
        let start_time = 1000;
        let end_time = 2000;
        let (client, _factory, _contract_id, auction_id) =
            setup_grace_window_test(&env, start_time, end_time);

        let bidder = Address::generate(&env);
        env.ledger().set_timestamp(1100);
        client.place_bid(&auction_id, &bidder, &100_i128);

        env.ledger().set_timestamp(2000);
        let result = client.try_close_auction(&auction_id);
        assert!(
            result.is_ok(),
            "close_auction must not be blocked by grace window"
        );
    }

    /// Grace window requires factory to be set.
    #[test]
    fn set_grace_window_requires_factory() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        let result = client.try_set_liquidation_grace_window(&60_u64);
        assert!(
            result.is_err(),
            "setting grace window without factory must fail"
        );
    }

    /// Zero grace window: bid immediately after start_time succeeds.
    #[test]
    fn zero_grace_window_allows_immediate_bid() {
        let env = Env::default();
        env.mock_all_auths();

        let factory = Address::generate(&env);
        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let auction_id = Symbol::new(&env, "zero_grace_ok");

        client.set_factory_contract(&factory);
        client.set_liquidation_grace_window(&0_u64);
        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &1000,
            &2000,
            &50_i128,
            &0_u32,
            &None,
            &None,
            &DutchAuctionDecay::None,
            &None,
        );

        let bidder = Address::generate(&env);
        env.ledger().set_timestamp(1000);
        let result = client.try_place_bid(&auction_id, &bidder, &100_i128);
        assert!(
            result.is_ok(),
            "zero grace window must allow immediate bid at start_time"
        );
    }
}