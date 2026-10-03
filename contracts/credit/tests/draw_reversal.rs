use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{token, Address, Env};

const DRAW_REVERSAL_WINDOW_SECS: u64 = 3600;

struct TestSetup {
    env: Env,
    client: CreditClient<'static>,
    admin: Address,
    borrower: Address,
    draw_ts: u64,
}

/// Deploys a mock token, wires it up as both the liquidity token/source and
/// (implicitly, via LiquidityToken) the collateral token, inits the contract,
/// opens a credit line, funds the borrower with enough collateral to satisfy
/// the 150% minimum ratio, and performs a single draw of `draw_amount`.
/// Returns the setup plus the ledger timestamp at which the draw occurred
/// (needed as `original_ts` for `reverse_draw`).
fn setup_draw_test(draw_amount: i128) -> TestSetup {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let borrower = Address::generate(&env);

    // Deploy a mock/stellar asset token to use as the liquidity token (which
    // doubles as the collateral token per storage::get_collateral_token).
    let token_admin = Address::generate(&env);
    let token_contract_id = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token_address = token_contract_id.address();
    let token_admin_client = token::StellarAssetClient::new(&env, &token_address);

    // 1. Init admin.
    client.init(&admin);

    // 2. Wire up liquidity token/source. The same token is used for
    // collateral accounting (see storage::get_collateral_token).
    client.set_liquidity_token(&token_address);
    client.set_liquidity_source(&contract_id);

    // Fund the contract's own address (acts as liquidity reserve) and the
    // borrower (for collateral deposit) with plenty of tokens.
    token_admin_client.mint(&contract_id, &1_000_000i128);
    token_admin_client.mint(&borrower, &1_000_000i128);

    // 3. Open credit line: (borrower, credit_limit, interest_rate_bps, risk_score)
    let credit_limit = 100_000i128;
    let interest_rate_bps = 500u32; // 5%
    let risk_score = 1u32;
    client.open_credit_line(&borrower, &credit_limit, &interest_rate_bps, &risk_score);

    // 4. Deposit collateral comfortably above the 150% minimum ratio for the
    // draw we're about to make.
    let collateral_amount = draw_amount * 3;
    client.deposit_collateral(&borrower, &collateral_amount);

    // 5. Draw credit — this also writes the DrawAudit record keyed by
    // (borrower, timestamp), which reverse_draw reads back as original_ts.
    client.draw_credit(&borrower, &draw_amount);
    let draw_ts = env.ledger().timestamp();

    TestSetup {
        env,
        client,
        admin,
        borrower,
        draw_ts,
    }
}

#[test]
fn test_draw_reversal_boundary_3600_and_3601() {
    let setup = setup_draw_test(1000);
    let reason_code = 0u32;

    // Boundary at exact cutoff (+3600s) -> should succeed.
    setup
        .env
        .ledger()
        .set_timestamp(setup.draw_ts + DRAW_REVERSAL_WINDOW_SECS);
    let res_exact =
        setup
            .client
            .try_reverse_draw(&setup.borrower, &100i128, &setup.draw_ts, &reason_code);
    assert!(
        res_exact.is_ok(),
        "Reversal at exactly {}s should succeed",
        DRAW_REVERSAL_WINDOW_SECS
    );

    // Boundary past cutoff (+3601s) -> should fail with DrawReversalWindowExpired.
    setup
        .env
        .ledger()
        .set_timestamp(setup.draw_ts + DRAW_REVERSAL_WINDOW_SECS + 1);
    let res_expired =
        setup
            .client
            .try_reverse_draw(&setup.borrower, &100i128, &setup.draw_ts, &reason_code);
    assert!(
        res_expired.is_err(),
        "Reversal at {}s must fail due to window expiry",
        DRAW_REVERSAL_WINDOW_SECS + 1
    );
}

#[test]
fn test_cumulative_partial_reversals_and_overlimit() {
    let setup = setup_draw_test(1000);
    let reason_code = 0u32;

    // First partial reversal (600 out of 1000).
    let res1 =
        setup
            .client
            .try_reverse_draw(&setup.borrower, &600i128, &setup.draw_ts, &reason_code);
    assert!(res1.is_ok(), "First partial reversal of 600 should succeed");

    // Second partial reversal (remaining 400 out of 1000).
    let res2 =
        setup
            .client
            .try_reverse_draw(&setup.borrower, &400i128, &setup.draw_ts, &reason_code);
    assert!(
        res2.is_ok(),
        "Second partial reversal of 400 should succeed"
    );

    // Exceeding original draw amount -> must fail with OverLimit.
    let res3 = setup
        .client
        .try_reverse_draw(&setup.borrower, &1i128, &setup.draw_ts, &reason_code);
    assert!(
        res3.is_err(),
        "Reversing beyond original draw amount must fail"
    );
}

#[test]
fn test_total_utilized_conserved_and_paused_protocol() {
    let setup = setup_draw_test(1000);
    let reason_code = 0u32;

    let initial_utilized = setup.client.get_total_utilized();
    let res =
        setup
            .client
            .try_reverse_draw(&setup.borrower, &500i128, &setup.draw_ts, &reason_code);

    assert!(res.is_ok(), "Reversal of 500 should succeed");
    assert_eq!(
        setup.client.get_total_utilized(),
        initial_utilized - 500i128,
        "Total utilized should decrease by exactly the reversed amount"
    );

    // Pause protocol as admin.
    setup.client.set_protocol_paused(&true);

    let paused_res =
        setup
            .client
            .try_reverse_draw(&setup.borrower, &100i128, &setup.draw_ts, &reason_code);
    assert!(
        paused_res.is_err(),
        "Paused protocol must reject reverse_draw calls"
    );
}
