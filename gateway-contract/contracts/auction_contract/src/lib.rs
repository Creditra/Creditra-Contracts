#![no_std]

mod events;

use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Env, Symbol};

use events::publish_bid_refunded_event;

#[contract]
pub struct Auction;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuctionState {
    pub bidder: Address,
    pub amount: i128,
}

#[contractimpl]
impl Auction {
    /// Place a bid in an English ascending-price auction.
    ///
    /// # Parameters
    ///
    /// * `env` — The Soroban contract environment.
    /// * `auction_id` — Symbol identifying the auction. Multiple auctions may coexist in persistent storage.
    /// * `bidder` — Address of the bidder. Must authorize this invocation; unauthorized calls panic.
    /// * `amount` — Bid amount in the auction's token denomination. Must be strictly positive and strictly greater than the current highest bid (if any).
    ///
    /// # Panics
    ///
    /// * If `amount <= 0` — bids must be positive.
    /// * If `amount <= previous_highest_bid.amount` — bids must strictly increase.
    ///
    /// # Behavior
    ///
    /// 1. **Authorization check**: `bidder.require_auth()` — panics if caller is not the bidder.
    /// 2. **Load previous highest bid** from persistent storage keyed by `auction_id`.
    /// 3. **If a previous bid exists**:
    ///    - Validate new bid is strictly higher (panics otherwise).
    ///    - **Emit** `BID_RFDN` event (via `publish_bid_refunded_event`) *before* transfer — ensures event ordering even if transfer fails.
    ///    - **Refund** previous bidder: if a `bid_token` address is stored in instance storage, transfers `prev.amount` from contract to `prev.bidder`. If no token is configured, refund is skipped (useful for test mocks).
    /// 4. **Store new highest bid**: overwrites `auction_id` key with `AuctionState { bidder, amount }`.
    ///
    /// # Refund Safety
    ///
    /// Event emission precedes the token transfer. If the transfer fails (e.g., insufficient contract balance, token panic), the event is already recorded, enabling off-chain reconciliation.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Alice bids 100 on auction "rare_nft"
    /// place_bid(env.clone(), symbol_short!("rare_nft"), alice_addr.clone(), 100);
    /// // State: { bidder: alice_addr, amount: 100 } stored under "rare_nft"
    ///
    /// // Bob bids 150 — Alice receives refund of 100, BID_RFDN event emitted
    /// place_bid(env.clone(), symbol_short!("rare_nft"), bob_addr.clone(), 150);
    /// // State: { bidder: bob_addr, amount: 150 }
    ///
    /// // Charlie attempts to bid 120 — panics (120 <= 150)
    /// // place_bid(env, symbol_short!("rare_nft"), charlie_addr, 120); // ❌ panic
    /// ```
    pub fn place_bid(env: Env, auction_id: Symbol, bidder: Address, amount: i128) {
        bidder.require_auth();

        if amount <= 0 {
            panic!("amount must be positive");
        }

        // Load existing highest bid if any
        let existing: Option<AuctionState> = env.storage().persistent().get(&auction_id);

        if let Some(prev) = existing {
            if amount <= prev.amount {
                panic!("bid must be higher than current highest bid");
            }

            // Emit refund event before performing token transfer
            publish_bid_refunded_event(&env, prev.bidder.clone(), prev.amount);

            // Attempt refund token transfer if token address configured in instance storage
            let token_addr: Option<Address> = env.storage().instance().get(&Symbol::new(&env, "bid_token"));
            if let Some(tkn) = token_addr {
                let token_client = token::Client::new(&env, &tkn);
                // Contract is the sender of refund transfers (for tests this will be mocked)
                token_client.transfer(&env.current_contract_address(), &prev.bidder, &prev.amount);
            }
        }

        // Store new highest bid
        let new_state = AuctionState { bidder: bidder.clone(), amount };
        env.storage().persistent().set(&auction_id, &new_state);
    }
}

#[cfg(test)]
mod test;
