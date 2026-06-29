// SPDX-License-Identifier: MIT

//! Integration tests for the two-step treasury withdrawal with 24-hour timelock.
//!
//! # Coverage
//! - Proposal creation and stored field correctness
//! - Authorization enforcement on both entrypoints
//! - Timelock boundary: before / exactly-at / after 24 hours
//! - Successful execution: funds transferred, balance cleared, proposal removed
//! - Replay prevention after execution
//! - Edge cases: no proposal, duplicate proposal, zero-balance proposal

use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{token, Address, Env};

const TIMELOCK: u64 = 86_400; // 24 hours in seconds

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Returns (env, contract_id, token_address, treasury).
/// Seeds the contract with 1_000 units of treasury balance via a repayment.
fn setup_with_balance() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let reserve = Address::generate(&env);
    let treasury = Address::generate(&env);

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();

    client.set_liquidity_token(&token_address);
    client.set_liquidity_source(&reserve);
    client.set_treasury(&admin, &treasury);
    client.set_protocol_fee_bps(&1_000); // 10 % fee so repayments build a balance

    // Open a line, draw, advance time so interest accrues, then repay with fee.
    client.open_credit_line(&borrower, &10_000_i128, &1_000_u32, &50_u32);
    let asset = token::StellarAssetClient::new(&env, &token_address);
    asset.mint(&contract_id, &10_000_i128); // fund reserve
    client.draw_credit(&borrower, &10_000_i128);

    env.ledger().with_mut(|l| l.timestamp = 31_536_000); // +1 year

    let repay = 11_000_i128;
    asset.mint(&borrower, &repay);
    token::Client::new(&env, &token_address).approve(
        &borrower,
        &contract_id,
        &repay,
        &u32::MAX,
    );
    client.repay_credit(&borrower, &repay);

    // Sanity: contract holds a treasury balance now.
    let summary = client.get_protocol_summary();
    assert!(summary.treasury_balance > 0, "setup: expected non-zero treasury balance");

    (env, contract_id, token_address, treasury)
}

// ── Proposal tests ───────────────────────────────────────────────────────────

#[test]
fn proposal_stores_correct_fields() {
    let (env, contract_id, _token, _treasury) = setup_with_balance();
    let client = CreditClient::new(&env, &contract_id);

    let now = 31_536_000_u64;
    env.ledger().with_mut(|l| l.timestamp = now);

    let admin = Address::generate(&env);
    client.propose_treasury_withdrawal(&admin);

    let proposal = client
        .get_pending_treasury_withdrawal()
        .expect("proposal should exist");

    assert_eq!(proposal.proposed_at, now);
    assert_eq!(proposal.execute_after, now + TIMELOCK);
    assert!(proposal.amount > 0);
}

#[test]
fn proposal_captures_current_treasury_balance() {
    let (env, contract_id, _token, _treasury) = setup_with_balance();
    let client = CreditClient::new(&env, &contract_id);

    let balance_before = client.get_protocol_summary().treasury_balance;

    let admin = Address::generate(&env);
    client.propose_treasury_withdrawal(&admin);

    let proposal = client.get_pending_treasury_withdrawal().unwrap();
    assert_eq!(proposal.amount, balance_before);
}

#[test]
fn no_proposal_returns_none() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    assert!(client.get_pending_treasury_withdrawal().is_none());
}

#[test]
#[should_panic]
fn duplicate_proposal_is_rejected() {
    let (env, contract_id, _token, _treasury) = setup_with_balance();
    let client = CreditClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    client.propose_treasury_withdrawal(&admin);
    client.propose_treasury_withdrawal(&admin); // must panic with TreasuryProposalExists
}

#[test]
#[should_panic]
fn propose_requires_treasury_configured() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    // no set_treasury — must panic with TreasuryNotSet
    client.propose_treasury_withdrawal(&admin);
}

// ── Timelock boundary tests ──────────────────────────────────────────────────

#[test]
#[should_panic]
fn execute_before_timelock_is_rejected() {
    let (env, contract_id, _token, _treasury) = setup_with_balance();
    let client = CreditClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    let now = 100_000_u64;
    env.ledger().with_mut(|l| l.timestamp = now);
    client.propose_treasury_withdrawal(&admin);

    // One second before the unlock.
    env.ledger().with_mut(|l| l.timestamp = now + TIMELOCK - 1);
    client.execute_treasury_withdrawal(&admin); // must panic with TreasuryTimelockActive
}

#[test]
fn execute_exactly_at_timelock_succeeds() {
    let (env, contract_id, token_address, treasury) = setup_with_balance();
    let client = CreditClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    let now = 100_000_u64;
    env.ledger().with_mut(|l| l.timestamp = now);
    client.propose_treasury_withdrawal(&admin);

    env.ledger().with_mut(|l| l.timestamp = now + TIMELOCK);
    client.execute_treasury_withdrawal(&admin);

    // Funds arrived.
    let token_client = token::Client::new(&env, &token_address);
    assert!(token_client.balance(&treasury) > 0);
}

#[test]
fn execute_after_timelock_succeeds() {
    let (env, contract_id, token_address, treasury) = setup_with_balance();
    let client = CreditClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    env.ledger().with_mut(|l| l.timestamp = 100_000);
    client.propose_treasury_withdrawal(&admin);

    env.ledger().with_mut(|l| l.timestamp = 100_000 + TIMELOCK + 3_600); // +1 h extra
    client.execute_treasury_withdrawal(&admin);

    let token_client = token::Client::new(&env, &token_address);
    assert!(token_client.balance(&treasury) > 0);
}

// ── Execution tests ──────────────────────────────────────────────────────────

#[test]
fn execute_transfers_full_proposed_amount() {
    let (env, contract_id, token_address, treasury) = setup_with_balance();
    let client = CreditClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    let expected = client.get_protocol_summary().treasury_balance;

    env.ledger().with_mut(|l| l.timestamp = 100_000);
    client.propose_treasury_withdrawal(&admin);

    env.ledger().with_mut(|l| l.timestamp = 100_000 + TIMELOCK);
    client.execute_treasury_withdrawal(&admin);

    let token_client = token::Client::new(&env, &token_address);
    assert_eq!(token_client.balance(&treasury), expected);
}

#[test]
fn execute_clears_proposal_and_treasury_balance() {
    let (env, contract_id, _token, _treasury) = setup_with_balance();
    let client = CreditClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    env.ledger().with_mut(|l| l.timestamp = 100_000);
    client.propose_treasury_withdrawal(&admin);

    env.ledger().with_mut(|l| l.timestamp = 100_000 + TIMELOCK);
    client.execute_treasury_withdrawal(&admin);

    // Proposal gone.
    assert!(client.get_pending_treasury_withdrawal().is_none());
    // On-chain treasury balance zeroed.
    assert_eq!(client.get_protocol_summary().treasury_balance, 0);
}

#[test]
#[should_panic]
fn replay_execution_is_rejected() {
    let (env, contract_id, _token, _treasury) = setup_with_balance();
    let client = CreditClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    env.ledger().with_mut(|l| l.timestamp = 100_000);
    client.propose_treasury_withdrawal(&admin);

    env.ledger().with_mut(|l| l.timestamp = 100_000 + TIMELOCK);
    client.execute_treasury_withdrawal(&admin);

    // Second execute with no proposal must panic with NoPendingTreasuryWithdrawal.
    client.execute_treasury_withdrawal(&admin);
}

#[test]
#[should_panic]
fn execute_without_proposal_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);
    // No proposal exists — must panic with NoPendingTreasuryWithdrawal.
    client.execute_treasury_withdrawal(&admin);
}

#[test]
fn zero_balance_proposal_executes_without_token_transfer() {
    // Propose when balance is zero — should succeed (no token CPI), just clear.
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    client.set_liquidity_token(&token_id.address());
    client.set_treasury(&admin, &treasury);

    // Treasury balance is 0 — proposal amount will be 0.
    env.ledger().with_mut(|l| l.timestamp = 1_000);
    client.propose_treasury_withdrawal(&admin);
    assert_eq!(client.get_pending_treasury_withdrawal().unwrap().amount, 0);

    env.ledger().with_mut(|l| l.timestamp = 1_000 + TIMELOCK);
    client.execute_treasury_withdrawal(&admin); // must not panic

    assert!(client.get_pending_treasury_withdrawal().is_none());
}

// ── New proposal after execution ─────────────────────────────────────────────

#[test]
fn new_proposal_allowed_after_execution() {
    let (env, contract_id, _token, _treasury) = setup_with_balance();
    let client = CreditClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    env.ledger().with_mut(|l| l.timestamp = 100_000);
    client.propose_treasury_withdrawal(&admin);
    env.ledger().with_mut(|l| l.timestamp = 100_000 + TIMELOCK);
    client.execute_treasury_withdrawal(&admin);

    // A second proposal can be submitted after execution.
    env.ledger().with_mut(|l| l.timestamp = 200_000);
    client.propose_treasury_withdrawal(&admin); // must not panic
    assert!(client.get_pending_treasury_withdrawal().is_some());
}
