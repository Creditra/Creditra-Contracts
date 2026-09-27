//! Regenerate `test_snapshots/cpu_baseline.json` from the current machine.
//!
//! CPU-time baselines are hardware-dependent (see `src/instrument.rs`), so
//! this file is not run automatically in CI. Maintainers run it manually
//! after an intentional performance change:
//!
//! ```bash
//! cargo run --features instrument --example cpu_baseline
//! git add contracts/creditra-credit/test_snapshots/cpu_baseline.json
//! ```

use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
use cosmwasm_std::Uint128;

use creditra_credit::contract::{execute, instantiate};
use creditra_credit::instrument::{
    entrypoint, write_baselines_to_manifest_dir, CpuBaseline, CpuSample, DEFAULT_ITERATIONS,
};
use creditra_credit::msg::{ExecuteMsg, InstantiateMsg};
use creditra_credit::penalties::{FlatFeeConfig, LateFeeConfig};

fn push(results: &mut Vec<CpuBaseline>, name: &'static str, sample: CpuSample) {
    eprintln!("{name:<24} cpu_nanos={}", sample.cpu_nanos);
    results.push(CpuBaseline::new(name, sample.cpu_nanos));
}

fn main() {
    let mut results: Vec<CpuBaseline> = Vec::new();
    let admin = "cpu_baseline_admin";
    let borrower = "cpu_baseline_borrower";

    // ── instantiate ──────────────────────────────────────────────────────────
    {
        let sample = CpuSample::measure_avg(DEFAULT_ITERATIONS, || {
            let mut deps = mock_dependencies();
            let owner = deps.api.addr_make(admin);
            instantiate(
                deps.as_mut(),
                mock_env(),
                message_info(&owner, &[]),
                InstantiateMsg {
                    owner: owner.to_string(),
                },
            )
            .unwrap();
        });
        push(&mut results, entrypoint::INSTANTIATE, sample);
    }

    // ── create_credit_line ───────────────────────────────────────────────────
    {
        let mut deps = mock_dependencies();
        let owner = deps.api.addr_make(admin);
        let borrower_addr = deps.api.addr_make(borrower);
        instantiate(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[]),
            InstantiateMsg {
                owner: owner.to_string(),
            },
        )
        .unwrap();
        let info = message_info(&owner, &[]);

        let sample = CpuSample::measure_avg(DEFAULT_ITERATIONS, || {
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
        push(&mut results, entrypoint::CREATE_CREDIT_LINE, sample);
    }

    // ── create_draw ──────────────────────────────────────────────────────────
    {
        let mut deps = mock_dependencies();
        let owner = deps.api.addr_make(admin);
        let borrower_addr = deps.api.addr_make(borrower);
        instantiate(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[]),
            InstantiateMsg {
                owner: owner.to_string(),
            },
        )
        .unwrap();
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

        let sample = CpuSample::measure_avg(DEFAULT_ITERATIONS, || {
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
        push(&mut results, entrypoint::CREATE_DRAW, sample);
    }

    // ── repay_draw ───────────────────────────────────────────────────────────
    {
        let mut deps = mock_dependencies();
        let owner = deps.api.addr_make(admin);
        let borrower_addr = deps.api.addr_make(borrower);
        instantiate(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[]),
            InstantiateMsg {
                owner: owner.to_string(),
            },
        )
        .unwrap();
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
        let mut next_draw = 0u64;

        let sample = CpuSample::measure_avg(DEFAULT_ITERATIONS, || {
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
        push(&mut results, entrypoint::REPAY_DRAW, sample);
    }

    // ── add_audit_memo ───────────────────────────────────────────────────────
    {
        let mut deps = mock_dependencies();
        let owner = deps.api.addr_make(admin);
        let borrower_addr = deps.api.addr_make(borrower);
        instantiate(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[]),
            InstantiateMsg {
                owner: owner.to_string(),
            },
        )
        .unwrap();
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

        let sample = CpuSample::measure_avg(DEFAULT_ITERATIONS, || {
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
        push(&mut results, entrypoint::ADD_AUDIT_MEMO, sample);
    }

    // ── update_protocol_version ──────────────────────────────────────────────
    {
        let mut deps = mock_dependencies();
        let owner = deps.api.addr_make(admin);
        instantiate(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[]),
            InstantiateMsg {
                owner: owner.to_string(),
            },
        )
        .unwrap();
        let info = message_info(&owner, &[]);
        let mut minor = 0u32;

        let sample = CpuSample::measure_avg(DEFAULT_ITERATIONS, || {
            minor += 1;
            execute(
                deps.as_mut(),
                mock_env(),
                info.clone(),
                ExecuteMsg::UpdateProtocolVersion { major: 1, minor },
            )
            .unwrap();
        });
        push(&mut results, entrypoint::UPDATE_PROTOCOL_VERSION, sample);
    }

    // ── set_oracle_quorum_config ─────────────────────────────────────────────
    {
        let mut deps = mock_dependencies();
        let owner = deps.api.addr_make(admin);
        instantiate(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[]),
            InstantiateMsg {
                owner: owner.to_string(),
            },
        )
        .unwrap();
        let info = message_info(&owner, &[]);

        let sample = CpuSample::measure_avg(DEFAULT_ITERATIONS, || {
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
        push(&mut results, entrypoint::SET_ORACLE_QUORUM_CONFIG, sample);
    }

    // ── submit_oracle_prices ─────────────────────────────────────────────────
    {
        let mut deps = mock_dependencies();
        let owner = deps.api.addr_make(admin);
        instantiate(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[]),
            InstantiateMsg {
                owner: owner.to_string(),
            },
        )
        .unwrap();
        let info = message_info(&owner, &[]);
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

        let sample = CpuSample::measure_avg(DEFAULT_ITERATIONS, || {
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
        push(&mut results, entrypoint::SUBMIT_ORACLE_PRICES, sample);
    }

    // ── set_late_fee_config ──────────────────────────────────────────────────
    {
        let mut deps = mock_dependencies();
        let owner = deps.api.addr_make(admin);
        instantiate(
            deps.as_mut(),
            mock_env(),
            message_info(&owner, &[]),
            InstantiateMsg {
                owner: owner.to_string(),
            },
        )
        .unwrap();
        let info = message_info(&owner, &[]);

        let sample = CpuSample::measure_avg(DEFAULT_ITERATIONS, || {
            execute(
                deps.as_mut(),
                mock_env(),
                info.clone(),
                ExecuteMsg::SetLateFeeConfig {
                    config: Some(LateFeeConfig::Flat(FlatFeeConfig {
                        amount: Uint128::new(100),
                    })),
                },
            )
            .unwrap();
        });
        push(&mut results, entrypoint::SET_LATE_FEE_CONFIG, sample);
    }

    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = write_baselines_to_manifest_dir(manifest_dir, &results);
    eprintln!("wrote {} baselines to {}", results.len(), path.display());
}
