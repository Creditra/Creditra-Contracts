#[cfg(test)]
mod tests {
    use super::super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::testutils::Events as _;
    use soroban_sdk::{Env, Symbol, TryFromVal, TryIntoVal};

    #[test]
    fn test_bid_refunded_event_emitted_when_outbid() {
        let env = Env::default();
        env.mock_all_auths();

        let alice = Address::generate(&env);
        let bob = Address::generate(&env);

        let contract_id = env.register(Auction, ());
        let client = AuctionClient::new(&env, &contract_id);

        // Alice places initial bid
        client.place_bid(&Symbol::new(&env, "auc1"), &alice, &100_i128);

        // Bob outbids Alice
        client.place_bid(&Symbol::new(&env, "auc1"), &bob, &200_i128);

        // Capture events and assert BID_RFDN event present with correct prev_bidder and amount
        let events = env.events().all();
        assert!(!events.is_empty());
        // Find the last BID_RFDN event
        let mut found = false;
        for (_contract, topics, data) in events.iter().rev() {
            let t0: Symbol = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
            if t0 == Symbol::new(&env, "BID_RFDN") {
                let event_data: events::BidRefundedEvent = data.try_into_val(&env).unwrap();
                assert_eq!(event_data.prev_bidder, alice);
                assert_eq!(event_data.amount, 100_i128);
                found = true;
                break;
            }
        }
        assert!(found, "BID_RFDN event not found");
    }

    /// Fuzz-style deterministic randomized sequence of bids.
    ///
    /// - Bounded iterations (100) for CI.
    /// - Fixed seed for reproducibility.
    /// - Ensures contract stored highest bid matches expected highest bid
    ///   and that BID_RFDN events are emitted with correct payload when outbid.
    #[test]
    fn fuzz_bid_sequences_deterministic() {
        use soroban_sdk::testutils::Address as _;
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

        // deterministic seed
        let mut seed: u64 = 0xdeadbeefcafebabe;
        // simple xorshift64* RNG
        fn next_u64(state: &mut u64) -> u64 {
            let mut x = *state;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *state = x;
            x
        }

        let mut expected: Option<(Address, i128)> = None;
        let iterations = 100usize; // bounded for CI
        let max_increment: i128 = 500;

        for _ in 0..iterations {
            let r = next_u64(&mut seed);
            let bidder_idx = (r as usize) % bidders.len();
            let bidder = bidders[bidder_idx].clone();

            // Generate an amount that is guaranteed to be higher than current by at least 1
            let base = expected.as_ref().map(|(_, a)| *a).unwrap_or(0_i128);
            let inc = ((next_u64(&mut seed) % (max_increment as u64)) as i128) + 1;
            let amount = base + inc;

            // place bid
            client.place_bid(&Symbol::new(&env, "fuzz_auc"), &bidder, &amount);

            // If there was a previous bidder, a BID_RFDN event should have been emitted
            if let Some((prev_addr, prev_amount)) = expected.clone() {
                // capture events and inspect the most recent BID_RFDN event
                let events = env.events().all();
                // Search backwards for a BID_RFDN event with the prev_amount
                let mut found = false;
                for (_c, topics, data) in events.iter().rev() {
                    let t0: Symbol = Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap();
                    if t0 == Symbol::new(&env, "BID_RFDN") {
                        let evt: events::BidRefundedEvent = data.try_into_val(&env).unwrap();
                        if evt.prev_bidder == prev_addr && evt.amount == prev_amount {
                            found = true;
                            break;
                        }
                    }
                }
                assert!(found, "expected BID_RFDN event for prev bidder");
            }

            // update expected highest
            expected = Some((bidder.clone(), amount));

            // assert on-chain stored highest bid matches expected (read inside contract context)
            let stored: Option<crate::AuctionState> = env.as_contract(&contract_id, || {
                env.storage().persistent().get(&Symbol::new(&env, "fuzz_auc"))
            });
            assert!(stored.is_some(), "stored state must exist");
            let s = stored.unwrap();
            assert_eq!(s.bidder, bidder);
            assert_eq!(s.amount, amount);
        }
    }
}
