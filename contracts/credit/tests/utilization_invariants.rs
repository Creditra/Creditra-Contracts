use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env};

fn setup<'a>(
    env: &'a Env,
    borrower: &'a Address,
    credit_limit: i128,
    reserve_balance: i128,
) -> (CreditClient<'a>, Address, Address) {
    env.mock_all_auths();

    let admin = Address::generate(env);
    let token_admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(token_admin);
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    client.set_liquidity_source(&contract_id);

    client.open_credit_line(borrower, &credit_limit, &300_u32, &70_u32);
    StellarAssetClient::new(env, &token_address).mint(&contract_id, &reserve_balance);

    (client, token_address, contract_id)
}

fn approve(env: &Env, token: &Address, owner: &Address, spender: &Address, amount: i128) {
    TokenClient::new(env, token).approve(owner, spender, &amount, &1_000_u32);
}

fn assert_non_negative_utilization(client: &CreditClient<'_>, borrower: &Address) {
    let line = client.get_credit_line(borrower).unwrap();
    assert!(
        line.utilized_amount >= 0,
        "utilized_amount must never be negative"
    );
}

fn assert_active_within_limit(client: &CreditClient<'_>, borrower: &Address) {
    let line = client.get_credit_line(borrower).unwrap();
    assert!(
        line.utilized_amount <= line.credit_limit,
        "active utilization must stay within credit_limit"
    );
    assert_non_negative_utilization(client, borrower);
}

#[test]
fn utilization_invariant_active_paths() {
    let env = Env::default();
    let borrower = Address::generate(&env);
    let (client, token, contract_id) = setup(&env, &borrower, 1_000, 2_500);

    for draw in [120_i128, 80, 300, 200] {
        client.draw_credit(&borrower, &draw);
        assert_active_within_limit(&client, &borrower);
    }

    StellarAssetClient::new(&env, &token).mint(&borrower, &2_000);
    approve(&env, &token, &borrower, &contract_id, 2_000);

    for repay in [50_i128, 400, 500, 600] {
        client.repay_credit(&borrower, &repay);
        assert_active_within_limit(&client, &borrower);
    }
}

#[test]
fn utilization_invariant_suspended_repay_floor() {
    let env = Env::default();
    let borrower = Address::generate(&env);
    let (client, token, contract_id) = setup(&env, &borrower, 1_000, 1_000);

    client.draw_credit(&borrower, &450);
    client.suspend_credit_line(&borrower);

    StellarAssetClient::new(&env, &token).mint(&borrower, &1_000);
    approve(&env, &token, &borrower, &contract_id, 1_000);

    for repay in [100_i128, 200, 700] {
        client.repay_credit(&borrower, &repay);
        let line = client.get_credit_line(&borrower).unwrap();
        assert!(line.utilized_amount >= 0);
    }
}

#[test]
fn utilization_invariant_defaulted_repay_floor() {
    let env = Env::default();
    let borrower = Address::generate(&env);
    let (client, token, contract_id) = setup(&env, &borrower, 1_000, 1_000);

    client.draw_credit(&borrower, &500);
    client.default_credit_line(&borrower);

    StellarAssetClient::new(&env, &token).mint(&borrower, &1_000);
    approve(&env, &token, &borrower, &contract_id, 1_000);

    for repay in [150_i128, 150, 800] {
        client.repay_credit(&borrower, &repay);
        let line = client.get_credit_line(&borrower).unwrap();
        assert!(line.utilized_amount >= 0);
    }
}
