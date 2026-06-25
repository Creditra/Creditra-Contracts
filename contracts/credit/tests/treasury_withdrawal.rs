// SPDX-License-Identifier: MIT

use creditra_credit::events::TreasuryWithdrawnEvent;
use creditra_credit::types::ContractError;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Events, Ledger};
use soroban_sdk::{token, Address, Env, Symbol, TryFromVal};

const WITHDRAWAL_DELAY: u64 = 86_400;
const YEAR: u64 = 31_557_600;

struct Setup {
    env: Env,
    contract_id: Address,
    token: Address,
    admin: Address,
    treasury: Address,
}

fn setup(configure_treasury: bool) -> Setup {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let reserve = Address::generate(&env);
    let treasury = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = token_id.address();
    let asset = token::StellarAssetClient::new(&env, &token);

    client.set_liquidity_token(&token);
    client.set_liquidity_source(&reserve);
    if configure_treasury {
        client.set_treasury(&admin, &treasury);
    }
    client.set_protocol_fee_bps(&1_000_u32);

    asset.mint(&reserve, &1_000_i128);
    asset.mint(&borrower, &1_600_i128);
    client.open_credit_line(&borrower, &1_000_i128, &1_000_u32, &50_u32);
    client.deposit_collateral(&borrower, &1_500_i128);
    client.draw_credit(&borrower, &1_000_i128);

    env.ledger().with_mut(|ledger| ledger.timestamp = YEAR);
    token::Client::new(&env, &token).approve(&borrower, &contract_id, &1_100_i128, &1_000_000_u32);
    client.repay_credit(&borrower, &1_100_i128);
    assert_eq!(client.get_treasury_balance(), 10);

    Setup {
        env,
        contract_id,
        token,
        admin,
        treasury,
    }
}

#[test]
fn proposal_records_amount_and_exact_24_hour_delay() {
    let setup = setup(true);
    let client = CreditClient::new(&setup.env, &setup.contract_id);

    client.propose_treasury_withdrawal(&6_i128);

    let pending = client.get_pending_treasury_withdrawal().unwrap();
    assert_eq!(pending.amount, 6);
    assert_eq!(pending.accept_after, YEAR + WITHDRAWAL_DELAY);
}

#[test]
fn confirm_reverts_one_second_before_delay() {
    let setup = setup(true);
    let client = CreditClient::new(&setup.env, &setup.contract_id);
    client.propose_treasury_withdrawal(&6_i128);

    setup
        .env
        .ledger()
        .with_mut(|ledger| ledger.timestamp = YEAR + WITHDRAWAL_DELAY - 1);
    let result = client.try_confirm_treasury_withdrawal();

    assert_eq!(
        result.err().unwrap().unwrap(),
        ContractError::AdminAcceptTooEarly.into()
    );
    assert_eq!(client.get_treasury_balance(), 10);
    assert!(client.get_pending_treasury_withdrawal().is_some());
}

#[test]
fn confirm_at_boundary_transfers_only_proposed_amount_and_clears_pending() {
    let setup = setup(true);
    let client = CreditClient::new(&setup.env, &setup.contract_id);
    let token_client = token::Client::new(&setup.env, &setup.token);
    client.propose_treasury_withdrawal(&6_i128);

    setup
        .env
        .ledger()
        .with_mut(|ledger| ledger.timestamp = YEAR + WITHDRAWAL_DELAY);
    client.confirm_treasury_withdrawal();

    assert_eq!(token_client.balance(&setup.treasury), 6);
    assert_eq!(client.get_treasury_balance(), 4);
    assert_eq!(client.get_pending_treasury_withdrawal(), None);
}

#[test]
fn failed_token_transfer_preserves_pending_state_and_accounting() {
    let setup = setup(true);
    let client = CreditClient::new(&setup.env, &setup.contract_id);
    let token_client = token::Client::new(&setup.env, &setup.token);
    let sink = Address::generate(&setup.env);
    client.propose_treasury_withdrawal(&6_i128);

    let contract_balance = token_client.balance(&setup.contract_id);
    setup.env.as_contract(&setup.contract_id, || {
        token_client.transfer(&setup.contract_id, &sink, &contract_balance);
    });

    setup
        .env
        .ledger()
        .with_mut(|ledger| ledger.timestamp = YEAR + WITHDRAWAL_DELAY);
    assert!(client.try_confirm_treasury_withdrawal().is_err());
    assert_eq!(client.get_treasury_balance(), 10);
    assert_eq!(client.get_pending_treasury_withdrawal().unwrap().amount, 6);

    token::StellarAssetClient::new(&setup.env, &setup.token).mint(&setup.contract_id, &6_i128);
    client.confirm_treasury_withdrawal();
    assert_eq!(client.get_treasury_balance(), 4);
    assert_eq!(client.get_pending_treasury_withdrawal(), None);
}

#[test]
fn latest_proposal_replaces_amount_and_restarts_delay() {
    let setup = setup(true);
    let client = CreditClient::new(&setup.env, &setup.contract_id);
    client.propose_treasury_withdrawal(&3_i128);

    setup
        .env
        .ledger()
        .with_mut(|ledger| ledger.timestamp = YEAR + 60);
    client.propose_treasury_withdrawal(&7_i128);

    let pending = client.get_pending_treasury_withdrawal().unwrap();
    assert_eq!(pending.amount, 7);
    assert_eq!(pending.accept_after, YEAR + 60 + WITHDRAWAL_DELAY);

    setup
        .env
        .ledger()
        .with_mut(|ledger| ledger.timestamp = YEAR + WITHDRAWAL_DELAY);
    let result = client.try_confirm_treasury_withdrawal();
    assert_eq!(
        result.err().unwrap().unwrap(),
        ContractError::AdminAcceptTooEarly.into()
    );

    setup
        .env
        .ledger()
        .with_mut(|ledger| ledger.timestamp = YEAR + 60 + WITHDRAWAL_DELAY);
    client.confirm_treasury_withdrawal();
    assert_eq!(client.get_treasury_balance(), 3);
}

#[test]
fn proposal_rejects_zero_negative_and_excess_amounts() {
    let setup = setup(true);
    let client = CreditClient::new(&setup.env, &setup.contract_id);

    for amount in [0_i128, -1_i128, 11_i128] {
        let result = client.try_propose_treasury_withdrawal(&amount);
        assert_eq!(
            result.err().unwrap().unwrap(),
            ContractError::InvalidAmount.into()
        );
    }
    assert_eq!(client.get_pending_treasury_withdrawal(), None);
}

#[test]
fn proposal_requires_configured_treasury() {
    let setup = setup(false);
    let client = CreditClient::new(&setup.env, &setup.contract_id);

    let result = client.try_propose_treasury_withdrawal(&1_i128);
    assert_eq!(
        result.err().unwrap().unwrap(),
        ContractError::TreasuryNotSet.into()
    );
}

#[test]
fn confirm_without_pending_proposal_reverts() {
    let setup = setup(true);
    let client = CreditClient::new(&setup.env, &setup.contract_id);

    let result = client.try_confirm_treasury_withdrawal();
    assert_eq!(
        result.err().unwrap().unwrap(),
        ContractError::Unauthorized.into()
    );
}

#[test]
#[should_panic]
fn proposal_requires_admin_auth() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    client.propose_treasury_withdrawal(&1_i128);
}

#[test]
#[should_panic]
fn confirmation_requires_admin_auth() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    client.confirm_treasury_withdrawal();
}

#[test]
fn successful_confirmation_emits_audit_event() {
    let setup = setup(true);
    let client = CreditClient::new(&setup.env, &setup.contract_id);
    client.propose_treasury_withdrawal(&6_i128);
    setup
        .env
        .ledger()
        .with_mut(|ledger| ledger.timestamp = YEAR + WITHDRAWAL_DELAY);
    client.confirm_treasury_withdrawal();

    let events = setup.env.events().all();
    let event = events.get(events.len() - 1).unwrap();
    assert_eq!(event.0, setup.contract_id);
    assert_eq!(
        Symbol::try_from_val(&setup.env, &event.1.get(0).unwrap()).unwrap(),
        Symbol::new(&setup.env, "credit")
    );
    assert_eq!(
        Symbol::try_from_val(&setup.env, &event.1.get(1).unwrap()).unwrap(),
        Symbol::new(&setup.env, "trs_wdraw")
    );

    let payload = TreasuryWithdrawnEvent::try_from_val(&setup.env, &event.2).unwrap();
    assert_eq!(
        payload,
        TreasuryWithdrawnEvent {
            treasury: setup.treasury,
            amount: 6,
            admin: setup.admin,
            timestamp: YEAR + WITHDRAWAL_DELAY,
        }
    );
}
