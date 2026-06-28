// boundless-events: test entry. Per-area test modules are mounted here.
//
// Tests are stubbed at the module level so that adding a new test file is a
// one-line change. Full test coverage matrix is documented in the
// boundless-platform-contract-prd.md Section 15.

#![cfg(test)]

mod admin;
mod bounty_pillar;
mod cancel_refund;
mod common;
mod contributions;
mod cross_contract;
mod crowdfunding;
mod escrow_fee_math;
mod grant_pillar;
mod hackathon_pillar;
mod handshake_focused;
mod token_whitelist;
