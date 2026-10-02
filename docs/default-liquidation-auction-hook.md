# Default Liquidation Auction Hook

## Scope

This document defines the minimal interface between:

- credit contract at contracts/credit
- auction contract at gateway-contract/contracts/auction_contract

for post-default liquidation handling.

## Interface

### Credit contract events and entrypoint

1. Event: credit/liq_req
- Emitted by default_credit_line.
- Payload: borrower, utilized_amount, timestamp.
- Purpose: signal that liquidation orchestration is required.

2. Entrypoint: settle_default_liquidation(borrower, recovered_amount, settlement_id, close_factor_bps)
- Admin-only.
- Accounting-only: no token transfer in this method.
- Parameters:
  - `close_factor_bps`: Basis points of utilized_amount that can be recovered in this settlement (1–10_000). Default 10_000 = full liquidation.
- Preconditions:
  - credit line status must be Defaulted
  - recovered_amount must be positive and <= target_recovery (utilized_amount * close_factor_bps / 10_000), except that a value of `0` is accepted when a configured auction contract reports a zero recovery (a closed auction with no bids)
  - settlement_id must be unused for that borrower
- Effects:
  - decreases utilized_amount by recovered_amount
  - when remaining utilized_amount == 0, status transitions to Closed
  - emits credit/liq_setl with close_factor_bps field
  - for a zero recovery, no accounting is applied: the line stays Defaulted with its debt unchanged, the settlement id is still consumed for replay protection, and credit/liq_norec is emitted instead of credit/liq_setl

### Auction contract settlement signal

Entrypoint: settle_default_liquidation(auction_id, credit_contract, borrower)
- Requires auction to be closed.
- One-time per auction_id.
- Emits LIQ_SETL/auction with auction_id, credit_contract, borrower, winner, recovered_amount.

For the complete protocol handshake details, required configuration, and failure modes, see the [Auction Contract Settlement Handshake](../gateway-contract/contracts/auction_contract/auction.md#settlement-handshake) documentation.

## Trust Boundaries

### On-chain

- Credit accounting authority remains in the credit contract.
- Settlement replay is prevented in both contracts by one-time settlement keys.
- Credit settlement never performs external token calls, preventing reentrancy through settlement.

### Off-chain

- Off-chain orchestrator listens for credit/liq_req and runs auction lifecycle.
- Off-chain process ensures auction proceeds are in protocol custody before calling credit settle_default_liquidation.
- Off-chain process maps auction_id to credit settlement_id deterministically.

## Security Notes

- The integration is intentionally event-driven and accounting-only at settlement time.
- No direct credit -> auction or auction -> credit value transfer path is introduced.
- This keeps settlement deterministic and avoids inconsistent partial accounting from failed token transfers.
