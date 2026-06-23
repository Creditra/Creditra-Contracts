use gateway_auction::{Auction, AuctionClient};

fn setup_auction(
    env: &Env,
    credit_id: &Address,
    auction_id: &Address,
    settlement_id: &Symbol,
    recovered_amount: i128,
) {
    let auction = AuctionClient::new(env, auction_id);
    auction.set_factory_contract(credit_id);

    let start_time = env.ledger().timestamp();
    let end_time = start_time + 1000;
    auction.init_auction(settlement_id, &start_time, &end_time, &100_i128);

    let bidder = Address::generate(env);
    auction.place_bid(settlement_id, &bidder, &recovered_amount);

    env.ledger().with_mut(|ledger| {
        ledger.timestamp = end_time;
    });
    auction.close_auction(settlement_id);
}

fn setup_defaulted_line(utilized_amount: i128) -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());

    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &1_000_000_i128);
    token::StellarAssetClient::new(&env, &token_address).mint(&borrower, &1_000_000_i128);
    token::Client::new(&env, &token_address).approve(
        &borrower,
        &contract_id,
        &1_000_000_i128,
        &1_000_000_u32,
    );

    client.open_credit_line(&borrower, &10_000, &300_u32, &60_u32);

    if utilized_amount > 0 {
        client.draw_credit(&borrower, &utilized_amount);
    }

    client.default_credit_line(&borrower);

    (env, contract_id, borrower)
}

fn has_event_topic(env: &Env, event_kind: &str) -> bool {
    let namespace = Symbol::new(env, "credit");
    let kind = Symbol::new(env, event_kind);

    for (_contract, topics, _data) in env.events().all().iter() {
        let t0: Symbol = Symbol::try_from_val(env, &topics.get(0).unwrap()).unwrap();
        let t1: Symbol = Symbol::try_from_val(env, &topics.get(1).unwrap()).unwrap();
        if t0 == namespace && t1 == kind {
            return true;
        }
    }

    false
}

#[test]
fn default_emits_liquidation_request_event() {
    let (env, _contract_id, _borrower) = setup_defaulted_line(500);

    assert!(has_event_topic(&env, "liq_req"));
}

#[test]
fn settle_partial_default_liquidation_and_block_replay() {
    let (env, contract_id, borrower) = setup_defaulted_line(1_000);
    let client = CreditClient::new(&env, &contract_id);
    let settlement_id = Symbol::new(&env, "auc_001");

    client.settle_default_liquidation(&borrower, &300_i128, &settlement_id, &None);
    assert!(has_event_topic(&env, "liq_setl"));

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Defaulted);
    assert_eq!(line.utilized_amount, 700);

    let replay = catch_unwind(AssertUnwindSafe(|| {
        client.settle_default_liquidation(&borrower, &50_i128, &settlement_id, &None);
    }));
    assert!(replay.is_err(), "replay settlement should panic");
}

#[test]
fn settle_full_default_liquidation_closes_credit_line() {
    let (env, contract_id, borrower) = setup_defaulted_line(450);
    let client = CreditClient::new(&env, &contract_id);

    client.settle_default_liquidation(&borrower, &450_i128, &Symbol::new(&env, "auc_fin"), &None);
    assert!(has_event_topic(&env, "closed"));
    assert!(has_event_topic(&env, "liq_setl"));

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Closed);
    assert_eq!(line.utilized_amount, 0);
}

#[test]
fn settle_default_liquidation_requires_defaulted_status() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    token::StellarAssetClient::new(&env, &token_address).mint(&contract_id, &1_000_000_i128);
    token::StellarAssetClient::new(&env, &token_address).mint(&borrower, &1_000_000_i128);
    token::Client::new(&env, &token_address).approve(
        &borrower,
        &contract_id,
        &1_000_000_i128,
        &1_000_000_u32,
    );

    client.open_credit_line(&borrower, &5_000, &200_u32, &40_u32);
    client.draw_credit(&borrower, &500_i128);

    let result = catch_unwind(AssertUnwindSafe(|| {
        client.settle_default_liquidation(&borrower, &100_i128, &Symbol::new(&env, "auc_bad"), &None);
    }));

    assert!(result.is_err(), "non-defaulted settlement should panic");
}

// ── Auction contract configuration ─────────────────────────────────────────

#[test]
fn set_and_get_auction_contract_address() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    // Initially no auction contract configured
    assert!(client.get_auction_contract().is_none());

    let auction_addr = Address::generate(&env);
    client.set_auction_contract(&auction_addr);

    assert_eq!(client.get_auction_contract().unwrap(), auction_addr);
}

#[test]
fn settle_with_auction_contract_configured_reduces_debt() {
    let (env, contract_id, borrower) = setup_defaulted_line(1_000);
    let client = CreditClient::new(&env, &contract_id);

    // Register and configure real auction contract address
    let auction_addr = env.register(Auction, ());
    client.set_auction_contract(&auction_addr);
    assert_eq!(client.get_auction_contract().unwrap(), auction_addr);

    // Setup the auction state and close it at 400 bid
    let settlement_id = Symbol::new(&env, "auc_cfg1");
    setup_auction(&env, &contract_id, &auction_addr, &settlement_id, 400_i128);

    // Settle partial — will atomically invoke the configured auction hook!
    client.settle_default_liquidation(&borrower, &400_i128, &settlement_id, &None);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Defaulted);
    assert_eq!(line.utilized_amount, 600);
    assert!(has_event_topic(&env, "liq_setl"));
}

#[test]
fn settle_full_with_auction_contract_closes_line() {
    let (env, contract_id, borrower) = setup_defaulted_line(800);
    let client = CreditClient::new(&env, &contract_id);

    // Register and configure real auction contract address
    let auction_addr = env.register(Auction, ());
    client.set_auction_contract(&auction_addr);

    // Setup the auction state and close it at 800 bid
    let settlement_id = Symbol::new(&env, "auc_full");
    setup_auction(&env, &contract_id, &auction_addr, &settlement_id, 800_i128);

    // Full settlement: recovered == utilized → should close line atomically
    client.settle_default_liquidation(&borrower, &800_i128, &settlement_id, &None);

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.status, CreditStatus::Closed);
    assert_eq!(line.utilized_amount, 0);
    assert!(has_event_topic(&env, "liq_setl"));
    assert!(has_event_topic(&env, "closed"));
}

#[test]
fn settle_clears_reentrancy_guard_on_success() {
    let (env, contract_id, borrower) = setup_defaulted_line(500);
    let client = CreditClient::new(&env, &contract_id);

    // First settlement — should set and clear reentrancy guard
    client.settle_default_liquidation(
        &borrower,
        &200_i128,
        &Symbol::new(&env, "auc_re1"),
        &None,
    );

    // Second settlement with different id — proves guard was cleared
    client.settle_default_liquidation(
        &borrower,
        &100_i128,
        &Symbol::new(&env, "auc_re2"),
        &None,
    );

    let line = client.get_credit_line(&borrower).unwrap();
    assert_eq!(line.utilized_amount, 200); // 500 - 200 - 100
}
