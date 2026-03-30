use soroban_sdk::{contracttype, symbol_short, Address, Env};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BidRefundedEvent {
    pub prev_bidder: Address,
    pub amount: i128,
}

pub fn publish_bid_refunded_event(env: &Env, prev_bidder: Address, amount: i128) {
    env.events()
        .publish((symbol_short!("BID_RFDN"), symbol_short!("auction")), BidRefundedEvent { prev_bidder, amount });
}
