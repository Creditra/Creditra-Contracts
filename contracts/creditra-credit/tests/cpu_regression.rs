//! Per-entrypoint CPU-time regression sampling.
//!
//! Measures wall-clock CPU time for every state-changing entrypoint and
//! compares each against `test_snapshots/cpu_baseline.json` when one has been
//! committed (see `examples/cpu_baseline.rs` to generate one). Until a
//! baseline exists, [`check_or_log_missing`] only logs the observed sample —
//! it never fails the build, since wall-clock timing is hardware-dependent
//! and a baseline generated on one machine should not gate CI on another.
//!
//! A generous sanity ceiling (not a tight tolerance) is also asserted per
//! entrypoint, to catch a genuine algorithmic blow-up (e.g. an accidentally
//! introduced O(n^2) loop) without being flaky.
//!
//! Requires the `instrument` Cargo feature (see `Cargo.toml`).

use cosmwasm_std::testing::{
    message_info, mock_dependencies, mock_env, MockApi, MockQuerier, MockStorage,
};
use cosmwasm_std::{Addr, OwnedDeps};

use creditra_credit::contract::{execute, instantiate};
use creditra_credit::instrument::{
    check_or_log_missing, entrypoint, load_baselines_from_manifest_dir, CpuSample,
};
use creditra_credit::msg::{ExecuteMsg, InstantiateMsg};
use creditra_credit::penalties::{FlatFeeConfig, LateFeeConfig};

/// Sanity ceiling: any single entrypoint call taking longer than this in a
/// mocked, in-memory test harness indicates a real algorithmic problem, not
/// hardware noise.
const SANITY_CEILING_NANOS: u64 = 50_000_000; // 50ms

const ITERATIONS: u32 = 20;

fn admin(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
    deps.api.addr_make("cpu_regression_admin")
}

fn borrower(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
    deps.api.addr_make("cpu_regression_borrower")
}

fn setup(deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>) {
    let env = mock_env();
    let owner = admin(deps);
    let info = message_info(&owner, &[]);
    instantiate(
        deps.as_mut(),
        env,
        info,
        InstantiateMsg {
            owner: owner.to_string(),
        },
    )
    .unwrap();
}

fn assert_sane(name: &str, sample: CpuSample) {
    assert!(
        sample.cpu_nanos < SANITY_CEILING_NANOS,
        "'{name}' took {} ns, exceeding the {SANITY_CEILING_NANOS} ns sanity ceiling",
        sample.cpu_nanos
    );
}

fn report(name: &str, sample: CpuSample) {
    assert_sane(name, sample);
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let baselines = load_baselines_from_manifest_dir(manifest_dir);
    check_or_log_missing(name, sample, &baselines);
}

#[test]
fn instantiate_cpu_sample() {
    let sample = CpuSample::measure_avg(ITERATIONS, || {
        let mut deps = mock_dependencies();
        setup(&mut deps);
    });
    report(entrypoint::INSTANTIATE, sample);
}

#[test]
fn create_credit_line_cpu_sample() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    let owner = admin(&deps);
    let borrower_addr = borrower(&deps);
    let info = message_info(&owner, &[]);

    let sample = CpuSample::measure_avg(ITERATIONS, || {
        execute(
            deps.as_mut(),
            mock_env(),
            info.clone(),
            ExecuteMsg::CreateCreditLine {
                borrower: borrower_addr.to_string(),
                collateral_denom: "ucollateral".to_string(),
                collateral_amount: "1000".to_string(),
                credit_denom: "ucredit".to_string(),
                credit_amount: "500".to_string(),
            },
        )
        .unwrap();
    });
    report(entrypoint::CREATE_CREDIT_LINE, sample);
}

#[test]
fn create_draw_cpu_sample() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    let owner = admin(&deps);
    let borrower_addr = borrower(&deps);
    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&owner, &[]),
        ExecuteMsg::CreateCreditLine {
            borrower: borrower_addr.to_string(),
            collateral_denom: "ucollateral".to_string(),
            collateral_amount: "1000".to_string(),
            credit_denom: "ucredit".to_string(),
            credit_amount: "500".to_string(),
        },
    )
    .unwrap();

    let info = message_info(&borrower_addr, &[]);
    let sample = CpuSample::measure_avg(ITERATIONS, || {
        execute(
            deps.as_mut(),
            mock_env(),
            info.clone(),
            ExecuteMsg::CreateDraw {
                credit_line_id: 0,
                amount: "10".to_string(),
                denom: "ucredit".to_string(),
            },
        )
        .unwrap();
    });
    report(entrypoint::CREATE_DRAW, sample);
}

#[test]
fn repay_draw_cpu_sample() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    let owner = admin(&deps);
    let borrower_addr = borrower(&deps);
    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&owner, &[]),
        ExecuteMsg::CreateCreditLine {
            borrower: borrower_addr.to_string(),
            collateral_denom: "ucollateral".to_string(),
            collateral_amount: "1000".to_string(),
            credit_denom: "ucredit".to_string(),
            credit_amount: "500".to_string(),
        },
    )
    .unwrap();

    let borrower_info = message_info(&borrower_addr, &[]);
    let mut next_draw: u64 = 0;
    let sample = CpuSample::measure_avg(ITERATIONS, || {
        execute(
            deps.as_mut(),
            mock_env(),
            borrower_info.clone(),
            ExecuteMsg::CreateDraw {
                credit_line_id: 0,
                amount: "1".to_string(),
                denom: "ucredit".to_string(),
            },
        )
        .unwrap();
        execute(
            deps.as_mut(),
            mock_env(),
            borrower_info.clone(),
            ExecuteMsg::RepayDraw {
                credit_line_id: 0,
                draw_id: next_draw,
            },
        )
        .unwrap();
        next_draw += 1;
    });
    report(entrypoint::REPAY_DRAW, sample);
}

#[test]
fn add_audit_memo_cpu_sample() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    let owner = admin(&deps);
    let borrower_addr = borrower(&deps);
    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&owner, &[]),
        ExecuteMsg::CreateCreditLine {
            borrower: borrower_addr.to_string(),
            collateral_denom: "ucollateral".to_string(),
            collateral_amount: "1000".to_string(),
            credit_denom: "ucredit".to_string(),
            credit_amount: "500".to_string(),
        },
    )
    .unwrap();
    execute(
        deps.as_mut(),
        mock_env(),
        message_info(&borrower_addr, &[]),
        ExecuteMsg::CreateDraw {
            credit_line_id: 0,
            amount: "10".to_string(),
            denom: "ucredit".to_string(),
        },
    )
    .unwrap();

    let owner_info = message_info(&owner, &[]);
    let sample = CpuSample::measure_avg(ITERATIONS, || {
        execute(
            deps.as_mut(),
            mock_env(),
            owner_info.clone(),
            ExecuteMsg::AddAuditMemo {
                credit_line_id: 0,
                draw_id: 0,
                memo: "routine note".to_string(),
            },
        )
        .unwrap();
    });
    report(entrypoint::ADD_AUDIT_MEMO, sample);
}

#[test]
fn update_protocol_version_cpu_sample() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    let info = message_info(&admin(&deps), &[]);

    let mut minor = 0u32;
    let sample = CpuSample::measure_avg(ITERATIONS, || {
        minor += 1;
        execute(
            deps.as_mut(),
            mock_env(),
            info.clone(),
            ExecuteMsg::UpdateProtocolVersion { major: 1, minor },
        )
        .unwrap();
    });
    report(entrypoint::UPDATE_PROTOCOL_VERSION, sample);
}

#[test]
fn set_oracle_quorum_config_cpu_sample() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    let info = message_info(&admin(&deps), &[]);

    let sample = CpuSample::measure_avg(ITERATIONS, || {
        execute(
            deps.as_mut(),
            mock_env(),
            info.clone(),
            ExecuteMsg::SetOracleQuorumConfig {
                min_quorum_k: 2,
                max_deviation_bps: 500,
                max_age_seconds: 3_600,
            },
        )
        .unwrap();
    });
    report(entrypoint::SET_ORACLE_QUORUM_CONFIG, sample);
}

#[test]
fn submit_oracle_prices_cpu_sample() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    let info = message_info(&admin(&deps), &[]);
    execute(
        deps.as_mut(),
        mock_env(),
        info.clone(),
        ExecuteMsg::SetOracleQuorumConfig {
            min_quorum_k: 2,
            max_deviation_bps: 500,
            max_age_seconds: 3_600,
        },
    )
    .unwrap();

    let sample = CpuSample::measure_avg(ITERATIONS, || {
        execute(
            deps.as_mut(),
            mock_env(),
            info.clone(),
            ExecuteMsg::SubmitOraclePrices {
                prices: vec![1_000, 1_010, 995],
            },
        )
        .unwrap();
    });
    report(entrypoint::SUBMIT_ORACLE_PRICES, sample);
}

#[test]
fn set_late_fee_config_cpu_sample() {
    let mut deps = mock_dependencies();
    setup(&mut deps);
    let info = message_info(&admin(&deps), &[]);

    let sample = CpuSample::measure_avg(ITERATIONS, || {
        execute(
            deps.as_mut(),
            mock_env(),
            info.clone(),
            ExecuteMsg::SetLateFeeConfig {
                config: Some(LateFeeConfig::Flat(FlatFeeConfig {
                    amount: cosmwasm_std::Uint128::new(100),
                })),
            },
        )
        .unwrap();
    });
    report(entrypoint::SET_LATE_FEE_CONFIG, sample);
}
