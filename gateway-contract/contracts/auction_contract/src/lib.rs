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
    /// Place a bid for an auction identified by `auction_id`.
    /// If there's a previous highest bidder, emit a `BID_RFDN` event
    /// before attempting the refund token transfer.
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
