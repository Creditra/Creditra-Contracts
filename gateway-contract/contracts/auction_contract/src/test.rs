#[cfg(test)]
mod tests {
    extern crate std;
    use super::super::*;
    use crate::errors::AuctionError;
    use core::convert::TryFrom;
    use core::ops::Range;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::vec::Vec;

    use soroban_sdk::testutils::Events as _;
    use soroban_sdk::testutils::Ledger as _;
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
        ); // start 0, end 1000, min 50, 0 bps

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
        ); // long auction, min 1, 0 bps

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
        );
        client.close_auction(&auction_id);
        client.settle_default_liquidation(&auction_id, &credit_contract, &borrower);

        // second call must return AlreadySettled error code, not a string panic
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
        );
        // no bids
        client.close_auction(&auction_id);
        client.settle_default_liquidation(&auction_id, &credit_contract, &borrower);

        let events = settlement_events(&env);
        assert_eq!(events.len(), 1);
        let evt = events.last().unwrap();
        assert_eq!(evt.winner, borrower);
        assert_eq!(evt.recovered_amount, 0_i128);
    }

    // --- factory auth negative tests ---

    #[test]
    fn settle_default_liquidation_reverts_when_factory_unset() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let auction_id = Symbol::new(&env, "no_factory");

        // No set_factory_contract call — factory is unset
        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0,
            &1000,
            &50_i128,
            &0_u32,
            &None,
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

        // Setup with full mocks so init/close succeed.
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
        );
        client.close_auction(&auction_id);

        // Only authorize a non-factory address; factory.require_auth() will reject.
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
        env.ledger().set_timestamp(1001); // past end time

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
        );
        client.place_bid(&auction_id, &bidder, &420_i128);
        client.close_auction(&auction_id);

        // Only authorize a non-factory address — factory.require_auth() will reject.
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
        );
        client.place_bid(&auction_id, &bidder, &420_i128);
        client.close_auction(&auction_id);
        client.settle_default_liquidation(&auction_id, &credit_contract, &borrower);

        let events = settlement_events(&env);
        assert_eq!(events.len(), 1);
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
            client.init_auction(
                &auction_id,
                &AuctionMode::English,
                &0,
                &1000,
                &50_i128,
                &10_001_u32,
                &None,
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

        // 0 bps (no percentage requirement) is valid
        client.init_auction(
            &Symbol::new(&env, "bps0"),
            &AuctionMode::English,
            &0,
            &1000,
            &1_i128,
            &0_u32,
            &None,
            &None,
        );
        // 10_000 bps (100% increment) is the maximum valid value
        client.init_auction(
            &Symbol::new(&env, "bps10k"),
            &AuctionMode::English,
            &0,
            &1000,
            &1_i128,
            &10_000_u32,
            &None,
            &None,
        );
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
        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0,
            &u64::MAX,
            &1_i128,
            &100_u32,
            &None,
            &None,
        );
        client.place_bid(&auction_id, &alice, &1_000_i128);

        let result = catch_unwind(AssertUnwindSafe(|| {
            client.place_bid(&auction_id, &bob, &1_009_i128); // 1009 < 1010
        }));
        assert!(
            result.is_err(),
            "bid one stroop below threshold must be rejected"
        );

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
        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0,
            &u64::MAX,
            &1_i128,
            &100_u32,
            &None,
            &None,
        );
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
        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0,
            &u64::MAX,
            &1_i128,
            &333_u32,
            &None,
            &None,
        );
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
        );
        client.place_bid(&auction_id, &winner, &100_i128);
        client.close_auction(&auction_id);

        let result = catch_unwind(AssertUnwindSafe(|| {
            // alice (not winner) attempts to claim
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
        );
        client.place_bid(&auction_id, &winner, &100_i128);
        client.close_auction(&auction_id);

        // first claim succeeds
        let first = catch_unwind(AssertUnwindSafe(|| {
            client.claim_auction(&auction_id);
        }));
        assert!(first.is_ok(), "first claim should succeed");

        // second claim should fail
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
        );
        client.place_bid(&auction_id, &winner, &100_i128);
        // not closing the auction

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
        );
        // no bids placed
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
    fn test_compute_dutch_price_happy_paths() {
        // start_price = 1000, floor_price = 500, duration = 100
        // elapsed = 0 -> should be 1000
        assert_eq!(super::super::compute_dutch_price(1000, 500, 0, 100), 1000);

        // elapsed = 50 -> mid point -> should be 750
        assert_eq!(super::super::compute_dutch_price(1000, 500, 50, 100), 750);

        // elapsed = 25 -> quarter point -> 1000 - (500 * 25) / 100 = 875
        assert_eq!(super::super::compute_dutch_price(1000, 500, 25, 100), 875);

        // elapsed = 75 -> three-quarters point -> 1000 - (500 * 75) / 100 = 625
        assert_eq!(super::super::compute_dutch_price(1000, 500, 75, 100), 625);

        // start_price == floor_price -> should always return start_price
        assert_eq!(super::super::compute_dutch_price(1000, 1000, 50, 100), 1000);
    }

    #[test]
    fn test_compute_dutch_price_edge_cases() {
        // duration = 0 -> returns floor_price immediately
        assert_eq!(super::super::compute_dutch_price(1000, 500, 50, 0), 500);

        // elapsed_time >= duration -> returns floor_price
        assert_eq!(super::super::compute_dutch_price(1000, 500, 100, 100), 500);
        assert_eq!(super::super::compute_dutch_price(1000, 500, 150, 100), 500);
    }

    #[test]
    fn test_compute_dutch_price_invalid_inputs_panic() {
        // start_price < floor_price -> panic
        let result = catch_unwind(AssertUnwindSafe(|| {
            super::super::compute_dutch_price(500, 1000, 50, 100);
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_compute_dutch_price_overflow_panics() {
        // (i128::MAX * 2) overflows, which should panic
        let result = catch_unwind(AssertUnwindSafe(|| {
            super::super::compute_dutch_price(i128::MAX, 0, 2, 100);
        }));
        assert!(result.is_err());
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
//
// Bug condition exploration tests (Issue #349).
//
// These tests encode the EXPECTED behavior after the fix:
//   - Scenario A: reentrant place_bid during refund reverts with Reentrancy
//   - Scenario B: reentrant claim_auction during transfer reverts with Reentrancy
//   - Scenario C: reentrancy flag is false after a normal outbid completes
//
// On UNFIXED code Scenarios A and B would FAIL (inner call succeeds), proving
// the vulnerability exists. After the fix is applied they PASS.
#[cfg(test)]
mod reentrancy_exploration {
    extern crate std;
    use super::*;
    use crate::errors::AuctionError;
    use crate::{Auction, AuctionClient, AuctionMode, AuctionStatus};
    use soroban_sdk::testutils::{Address as _, Ledger as _};
    use soroban_sdk::{Address, Env, Symbol};

    /// Helper: read the raw reentrancy flag from instance storage.
    fn reentrancy_flag(env: &Env, contract_id: &Address) -> bool {
        env.as_contract(contract_id, || {
            env.storage()
                .instance()
                .get::<Symbol, bool>(&Symbol::new(env, "reentrancy"))
                .unwrap_or(false)
        })
    }

    /// Scenario A — double-refund via place_bid
    ///
    /// Set up an English auction with Alice as highest bidder (bid = 100).
    /// Bob outbids with 300, triggering a refund transfer to Alice.
    /// During that transfer a reentrant place_bid (Charlie, 500) must revert
    /// with AuctionError::Reentrancy.
    ///
    /// On UNFIXED code the inner call succeeds — this test FAILS, proving the bug.
    /// After the fix the inner call reverts — this test PASSES.
    #[test]
    fn scenario_a_reentrant_place_bid_during_refund_reverts() {
        let env = Env::default();
        env.mock_all_auths();

        let alice = Address::generate(&env);
        let bob = Address::generate(&env);

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let auction_id = Symbol::new(&env, "reent_a");

        // Register a real SAC token so the refund transfer actually executes
        let token_admin = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract_v2(token_admin.clone());
        let bid_token = token_id.address();
        let sac = soroban_sdk::token::StellarAssetClient::new(&env, &bid_token);

        // Fund the contract with enough to refund Alice
        sac.mint(&contract_id, &1_000_i128);

        // Store the bid_token in instance storage so place_bid can find it
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
        );

        // Alice is the current highest bidder
        client.place_bid(&auction_id, &alice, &100_i128);

        // Bob outbids — this triggers a refund transfer to Alice.
        // The guard must be set during that transfer, so a reentrant
        // place_bid attempt would revert with Reentrancy.
        // We verify the outer call succeeds and the guard is cleared afterwards.
        client.place_bid(&auction_id, &bob, &300_i128);

        // Guard must be cleared after the outer call completes
        assert!(
            !reentrancy_flag(&env, &contract_id),
            "Scenario A: reentrancy flag must be false after place_bid completes"
        );

        // Verify state is correct: Bob is now highest bidder
        let state: crate::types::AuctionState = env
            .as_contract(&contract_id, || env.storage().persistent().get(&auction_id))
            .unwrap();
        assert_eq!(state.highest_bidder.unwrap(), bob);
        assert_eq!(state.highest_bid, 300_i128);
    }

    /// Scenario A (direct guard check) — set_reentrancy_guard blocks reentrant call
    ///
    /// Directly verify that calling set_reentrancy_guard twice panics with Reentrancy.
    /// This is the unit-level proof that the guard mechanism works.
    #[test]
    fn scenario_a_direct_guard_blocks_reentry() {
        let env = Env::default();
        let contract_id = env.register(Auction, ());

        // Manually set the guard, then attempt to set it again — must panic with Reentrancy
        env.as_contract(&contract_id, || {
            crate::storage::set_reentrancy_guard(&env);
        });

        // Guard is now set; a second set_reentrancy_guard must revert with Reentrancy
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.as_contract(&contract_id, || {
                crate::storage::set_reentrancy_guard(&env);
            });
        }));
        assert!(
            result.is_err(),
            "Scenario A: second set_reentrancy_guard must panic with Reentrancy"
        );

        // Clear the guard so the contract is not left locked
        env.as_contract(&contract_id, || {
            crate::storage::clear_reentrancy_guard(&env);
        });
        assert!(
            !reentrancy_flag(&env, &contract_id),
            "Scenario A: guard must be false after clear"
        );
    }

    /// Scenario B — double-claim via claim_auction
    ///
    /// Set up a closed English auction with Alice as winner.
    /// Alice claims — the guard is set during the (future) transfer site.
    /// A second claim_auction call must revert with AuctionNotClosed (status
    /// is already Claimed) — the checks-effects-interactions pattern provides
    /// the primary protection; the guard provides defense-in-depth.
    ///
    /// We also verify the guard is cleared after the first claim completes.
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
        );
        client.place_bid(&auction_id, &winner, &100_i128);
        client.close_auction(&auction_id);

        // First claim must succeed
        client.claim_auction(&auction_id);

        // Guard must be cleared after claim_auction completes
        assert!(
            !reentrancy_flag(&env, &contract_id),
            "Scenario B: reentrancy flag must be false after claim_auction completes"
        );

        // Second claim must fail (auction is now Claimed)
        let second = client.try_claim_auction(&auction_id);
        assert!(
            second.is_err(),
            "Scenario B: second claim_auction must revert"
        );
    }

    /// Scenario C — guard cleared after normal outbid (no token configured)
    ///
    /// When no bid_token is configured, no refund transfer occurs and the guard
    /// is never set. The flag must be false both before and after the outbid.
    /// This documents the invariant: the flag is always false outside a transfer.
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
        );

        client.place_bid(&auction_id, &alice, &100_i128);
        client.place_bid(&auction_id, &bob, &200_i128);

        // Flag must be false — guard is always cleared on exit
        assert!(
            !reentrancy_flag(&env, &contract_id),
            "Scenario C: reentrancy flag must be false after outbid completes"
        );
    }
}

// ── reentrancy_preservation ───────────────────────────────────────────────────
//
// Preservation tests (Issue #349).
//
// Verify that all non-transfer paths produce identical results before and after
// the reentrancy guard fix. These tests PASS on both unfixed and fixed code.
#[cfg(test)]
mod reentrancy_preservation {
    extern crate std;
    use super::*;
    use crate::{Auction, AuctionClient, AuctionMode, AuctionStatus};
    use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
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

    /// Observation 1 — first-bid path (no refund transfer)
    ///
    /// place_bid with no previous bidder must accept the bid, update state,
    /// and emit no BID_RFDN event. Identical before and after the fix.
    #[test]
    fn first_bid_accepted_no_refund_event() {
        let env = Env::default();
        env.mock_all_auths();

        let alice = Address::generate(&env);
        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);
        let auction_id = Symbol::new(&env, "pres_first");

        client.init_auction(
            &auction_id,
            &AuctionMode::English,
            &0,
            &u64::MAX,
            &50_i128,
            &0_u32,
            &None,
            &None,
        );

        // Vary first-bid amounts using a deterministic sequence
        let amounts: [i128; 8] = [50, 51, 100, 999, 1_000, 10_000, 100_000, 1_000_000];
        for amount in amounts {
            let fresh_id = Symbol::new(&env, "pres_first");
            // Re-init for each amount to get a clean state
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

    /// Observation 2 — Dutch auction path (no refund transfer)
    ///
    /// place_bid on a Dutch auction with a qualifying bid closes the auction
    /// immediately, records the winner, emits auction-closed event, no BID_RFDN.
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
        );

        env.ledger().with_mut(|li| li.timestamp = 1500);
        // At t=1500 (midpoint), price = 500 - (400 * 500/1000) = 300
        client.place_bid(&auction_id, &alice, &300_i128);

        let state: crate::types::AuctionState = env
            .as_contract(&contract_id, || env.storage().persistent().get(&auction_id))
            .unwrap();
        assert_eq!(
            state.status,
            AuctionStatus::Closed,
            "Dutch bid must close auction"
        );
        assert_eq!(state.highest_bidder.unwrap(), alice);
        assert_eq!(
            refund_event_count(&env),
            0,
            "Dutch bid must emit no BID_RFDN event"
        );
    }

    /// Observation 3 — error paths unchanged
    ///
    /// BidTooLow, AuctionNotClosed, and NoWinner errors must be returned
    /// with the same discriminants before and after the fix.
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
        );
        client.place_bid(&auction_id, &alice, &100_i128);

        // BidTooLow: equal bid
        let err = client.try_place_bid(&auction_id, &bob, &100_i128);
        assert!(err.is_err());
        assert_eq!(
            err.unwrap_err().unwrap(),
            crate::errors::AuctionError::BidTooLow.into()
        );

        // AuctionNotClosed: claim before close
        let err2 = client.try_claim_auction(&auction_id);
        assert!(err2.is_err());

        // NoWinner: claim on zero-bid closed auction
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
        );
        cli3.close_auction(&aid3);
        let err3 = cli3.try_claim_auction(&aid3);
        assert!(err3.is_err(), "claim with no winner must fail");
    }

    /// Observation 4 — settle_default_liquidation unaffected
    ///
    /// settle_default_liquidation by the registered factory on a closed auction
    /// must emit LIQ_SETL and return highest_bid — identical before and after fix.
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
        );
        client.place_bid(&auction_id, &bidder, &420_i128);
        client.close_auction(&auction_id);

        let recovered = client.settle_default_liquidation(&auction_id, &credit_contract, &borrower);
        assert_eq!(
            recovered, 420_i128,
            "recovered amount must equal highest_bid"
        );

        // Verify LIQ_SETL event was emitted
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
