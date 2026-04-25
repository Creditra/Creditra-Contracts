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

2. Entrypoint: settle_default_liquidation(borrower, recovered_amount, settlement_id)
- Admin-only.
- Accounting-only: no token transfer in this method.
- Preconditions:
  - credit line status must be Defaulted
  - recovered_amount must be positive and <= utilized_amount
  - settlement_id must be unused for that borrower
- Effects:
  - decreases utilized_amount by recovered_amount
  - when remaining utilized_amount == 0, status transitions to Closed
  - emits credit/liq_setl

### Auction contract settlement signal

Entrypoint: settle_default_liquidation(auction_id, credit_contract, borrower)
- Requires auction to be closed.
- One-time per auction_id.
- Emits LIQ_SETL/auction with auction_id, credit_contract, borrower, winner, recovered_amount.

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
