use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BidRefundedEvent {
    pub prev_bidder: Address,
    pub amount: i128,
}

pub fn publish_bid_refunded_event(env: &Env, prev_bidder: Address, amount: i128) {
    env.events().publish(
        (symbol_short!("BID_RFDN"), symbol_short!("auction")),
        BidRefundedEvent {
            prev_bidder,
            amount,
        },
    );
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DefaultLiquidationSettlementEvent {
    pub auction_id: Symbol,
    pub credit_contract: Address,
    pub borrower: Address,
    pub winner: Address,
    pub recovered_amount: i128,
}

pub fn publish_default_liquidation_settlement_event(
    env: &Env,
    auction_id: Symbol,
    credit_contract: Address,
    borrower: Address,
    winner: Address,
    recovered_amount: i128,
) {
    env.events().publish(
        (symbol_short!("LIQ_SETL"), symbol_short!("auction")),
        DefaultLiquidationSettlementEvent {
            auction_id,
            credit_contract,
            borrower,
            winner,
            recovered_amount,
        },
    );
}
