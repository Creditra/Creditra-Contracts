#![cfg(test)]

use proptest::prelude::*;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env, Symbol};

use gateway_auction::{
    Auction, AuctionClient, AuctionMode, AuctionState, AuctionStatus, DutchAuctionDecay,
};

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    #[test]
    fn test_auction_state_invariants(
        start_time in 100_000u64..200_000u64,
        duration in 1u64..100_000u64,
        min_bid in 1i128..100_000i128,
        min_increment_bps in 0u32..10_000u32,
        is_dutch in any::<bool>(),
        dutch_start_price in 100_000i128..200_000i128,
        dutch_floor_price in 1i128..100_000i128,
        decay_idx in 0u8..3u8,
        step_count in 1u32..100u32,
    ) {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, Auction);
        let client = AuctionClient::new(&env, &contract_id);

        // `init_auction` is a factory-gated mutation: register the factory so
        // the create call is authorized.
        let factory = Address::generate(&env);
        client.set_factory_contract(&factory);

        let auction_id = Symbol::new(&env, "prop_auc");
        let end_time = start_time + duration;

        let mode = if is_dutch { AuctionMode::Dutch } else { AuctionMode::English };
        let decay = match decay_idx {
            0 => DutchAuctionDecay::None,
            1 => DutchAuctionDecay::Linear,
            2 => DutchAuctionDecay::Stepped,
            _ => DutchAuctionDecay::Exponential,
        };

        // Enforce invariants for parameters before calling init_auction
        let start_price = dutch_start_price.max(min_bid).max(dutch_floor_price);
        let d_start = if is_dutch { Some(start_price) } else { None };
        let d_floor = if is_dutch { Some(dutch_floor_price) } else { None };
        let d_steps = if is_dutch && decay == DutchAuctionDecay::Stepped { Some(step_count) } else { None };

        // Should not panic
        client.init_auction(
            &auction_id,
            &mode,
            &start_time,
            &end_time,
            &min_bid,
            &min_increment_bps,
            &d_start,
            &d_floor,
            &Some(decay.clone()),
            &d_steps,
        );

        // Verify the persisted state invariants directly via storage
        let state: AuctionState = env.as_contract(&contract_id, || {
            env.storage().persistent().get(&auction_id).unwrap()
        });

        // Invariant 1: Status starts as Open
        assert_eq!(state.status, AuctionStatus::Open);

        // Invariant 2: Start time < End time
        assert!(state.config.start_time < state.config.end_time);

        // Invariant 3: min_increment_bps <= 10_000
        assert!(state.config.min_increment_bps <= 10_000);

        // Invariant 4: highest bid is initially 0 and bidder is None
        assert_eq!(state.highest_bid, 0);
        assert!(state.highest_bidder.is_none());

        // Invariant 5: Mode-specific invariants
        if state.config.mode == AuctionMode::Dutch {
            let sp = state.config.dutch_start_price.unwrap();
            let fp = state.config.dutch_floor_price.unwrap();
            assert!(sp >= fp);
            assert!(sp >= state.config.min_bid);

            if state.config.dutch_decay == DutchAuctionDecay::Stepped {
                assert!(state.config.dutch_step_count.unwrap() > 0);
            }
        }
    }
}
