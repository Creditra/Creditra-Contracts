// SPDX-License-Identifier: MIT

//! # Late-fee overflow and boundary test suite
//!
//! Verifies late-fee arithmetic under extreme boundaries and state transitions
//! to satisfy Issue #1126 acceptance criteria:
//!
//! | Module | Acceptance Criterion |
//! |---|---|
//! | [`overflow_protection`] | Overflow returns a stable error before mutation |
//! | [`boundary_conditions`] | Zero and maximum boundaries are tested |
//! | [`state_transition_and_clearing`] | Switching fee modes cannot carry stale state |
//! | [`golden_tests`] | Golden tests cover all public fee variants |
//! | [`authorization_boundaries`] | Unauthorized actors cannot alter late-fee state |

use cosmwasm_std::{
    from_json,
    testing::{message_info, mock_dependencies, mock_env, MockApi, MockQuerier, MockStorage},
    Addr, OwnedDeps, Storage, Uint128,
};
use creditra_credit::{
    contract::{execute, instantiate, query},
    error::ContractError,
    msg::{ExecuteMsg, InstantiateMsg, LateFeeConfigResponse, QueryMsg},
    penalties::{
        compute_apr_late_fee, compute_late_fee, compute_late_fee_optional,
        validate_late_fee_config, AprFeeConfig, FlatFeeConfig, LateFeeConfig, MAX_SURCHARGE_BPS,
    },
    state::LATE_FEE_CONFIG,
};

fn admin(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
    deps.api.addr_make("admin")
}

fn attacker(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Addr {
    deps.api.addr_make("attacker")
}

fn setup(deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>) {
    let env = mock_env();
    let admin_addr = admin(deps);
    let info = message_info(&admin_addr, &[]);
    let msg = InstantiateMsg {
        owner: admin_addr.to_string(),
    };
    instantiate(deps.as_mut(), env, info, msg).unwrap();
}

fn query_late_fee(deps: &OwnedDeps<MockStorage, MockApi, MockQuerier>) -> Option<LateFeeConfig> {
    let env = mock_env();
    let raw = query(deps.as_ref(), env, QueryMsg::GetLateFeeConfig {}).unwrap();
    let resp: LateFeeConfigResponse = from_json(&raw).unwrap();
    resp.config
}

fn set_late_fee(
    deps: &mut OwnedDeps<MockStorage, MockApi, MockQuerier>,
    sender: &Addr,
    config: Option<LateFeeConfig>,
) -> Result<(), ContractError> {
    let env = mock_env();
    let info = message_info(sender, &[]);
    let msg = ExecuteMsg::SetLateFeeConfig { config };
    execute(deps.as_mut(), env, info, msg).map(|_| ())
}

mod overflow_protection {
    use super::*;

    #[test]
    fn flat_multiplication_overflow_returns_stable_error() {
        let config = LateFeeConfig::Flat(FlatFeeConfig {
            amount: Uint128::MAX,
        });
        let res = compute_late_fee(config, 2);
        assert_eq!(res, Err(ContractError::Overflow));
    }

    #[test]
    fn flat_overflow_with_max_u64_installments_returns_stable_error() {
        let config = LateFeeConfig::Flat(FlatFeeConfig {
            amount: Uint128::MAX,
        });
        let res = compute_late_fee(config, u64::MAX);
        assert_eq!(res, Err(ContractError::Overflow));
    }

    #[test]
    fn apr_penalty_overflow_returns_stable_error() {
        let config = AprFeeConfig {
            surcharge_bps: MAX_SURCHARGE_BPS,
        };
        let res = compute_apr_late_fee(
            Uint128::MAX,
            &config,
            creditra_credit::accrual::SECONDS_PER_YEAR,
        );
        assert_eq!(res, Err(ContractError::Overflow));
    }

    #[test]
    fn overflow_prevents_simulated_state_mutation() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let admin_addr = admin(&deps);

        let initial_config = LateFeeConfig::Flat(FlatFeeConfig {
            amount: Uint128::new(500),
        });
        set_late_fee(&mut deps, &admin_addr, Some(initial_config)).unwrap();

        let initial_snapshot = deps.storage.get(b"lfc").expect("must be present");

        let mut calculation = || -> Result<Uint128, ContractError> {
            let fee = compute_late_fee(
                LateFeeConfig::Flat(FlatFeeConfig {
                    amount: Uint128::MAX,
                }),
                2,
            )?;
            let mutated_config = LateFeeConfig::Flat(FlatFeeConfig { amount: fee });
            LATE_FEE_CONFIG.save(&mut deps.storage, &mutated_config)?;
            Ok(fee)
        };

        let result = calculation();
        assert_eq!(result, Err(ContractError::Overflow));

        let current_snapshot = deps.storage.get(b"lfc").expect("must still be present");
        assert_eq!(initial_snapshot, current_snapshot);
        assert_eq!(query_late_fee(&deps), Some(initial_config));
    }
}

mod boundary_conditions {
    use super::*;

    #[test]
    fn zero_missed_installments_returns_zero_for_all_configs() {
        let flat_small = LateFeeConfig::Flat(FlatFeeConfig {
            amount: Uint128::new(1),
        });
        let flat_max = LateFeeConfig::Flat(FlatFeeConfig {
            amount: Uint128::MAX,
        });
        let apr_zero = LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 0 });
        let apr_max = LateFeeConfig::AprBased(AprFeeConfig {
            surcharge_bps: MAX_SURCHARGE_BPS,
        });

        assert_eq!(compute_late_fee(flat_small, 0).unwrap(), Uint128::zero());
        assert_eq!(compute_late_fee(flat_max, 0).unwrap(), Uint128::zero());
        assert_eq!(compute_late_fee(apr_zero, 0).unwrap(), Uint128::zero());
        assert_eq!(compute_late_fee(apr_max, 0).unwrap(), Uint128::zero());
        assert_eq!(compute_late_fee_optional(None, 0).unwrap(), Uint128::zero());
    }

    #[test]
    fn single_installment_at_maximum_uint128_succeeds() {
        let flat_max = LateFeeConfig::Flat(FlatFeeConfig {
            amount: Uint128::MAX,
        });
        assert_eq!(compute_late_fee(flat_max, 1).unwrap(), Uint128::MAX);
    }

    #[test]
    fn maximum_u64_installments_at_unit_amount_succeeds() {
        let flat_unit = LateFeeConfig::Flat(FlatFeeConfig {
            amount: Uint128::new(1),
        });
        assert_eq!(
            compute_late_fee(flat_unit, u64::MAX).unwrap(),
            Uint128::from(u64::MAX)
        );
    }

    #[test]
    fn maximum_safe_product_and_one_step_above() {
        let half_max = Uint128::MAX / Uint128::new(2);
        let safe_fee =
            compute_late_fee(LateFeeConfig::Flat(FlatFeeConfig { amount: half_max }), 2).unwrap();
        assert_eq!(safe_fee, half_max.checked_mul(Uint128::new(2)).unwrap());

        let half_max_plus_one = half_max + Uint128::new(1);
        let overflow_err = compute_late_fee(
            LateFeeConfig::Flat(FlatFeeConfig {
                amount: half_max_plus_one,
            }),
            2,
        )
        .unwrap_err();
        assert_eq!(overflow_err, ContractError::Overflow);
    }

    #[test]
    fn zero_flat_amount_rejected_in_validation_and_set() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let admin_addr = admin(&deps);

        let zero_config = LateFeeConfig::Flat(FlatFeeConfig {
            amount: Uint128::zero(),
        });
        assert_eq!(
            validate_late_fee_config(&zero_config).unwrap_err(),
            ContractError::InvalidAmount
        );
        let err = set_late_fee(&mut deps, &admin_addr, Some(zero_config)).unwrap_err();
        assert_eq!(err, ContractError::InvalidAmount);
    }

    #[test]
    fn surcharge_bps_boundaries_tested() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let admin_addr = admin(&deps);

        let zero_bps = LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 0 });
        assert!(validate_late_fee_config(&zero_bps).is_ok());
        set_late_fee(&mut deps, &admin_addr, Some(zero_bps)).unwrap();
        assert_eq!(query_late_fee(&deps), Some(zero_bps));

        let max_bps = LateFeeConfig::AprBased(AprFeeConfig {
            surcharge_bps: MAX_SURCHARGE_BPS,
        });
        assert!(validate_late_fee_config(&max_bps).is_ok());
        set_late_fee(&mut deps, &admin_addr, Some(max_bps)).unwrap();
        assert_eq!(query_late_fee(&deps), Some(max_bps));

        let above_max_bps = LateFeeConfig::AprBased(AprFeeConfig {
            surcharge_bps: MAX_SURCHARGE_BPS + 1,
        });
        assert_eq!(
            validate_late_fee_config(&above_max_bps).unwrap_err(),
            ContractError::RateTooHigh
        );
        assert_eq!(
            set_late_fee(&mut deps, &admin_addr, Some(above_max_bps)).unwrap_err(),
            ContractError::RateTooHigh
        );

        let max_u32_bps = LateFeeConfig::AprBased(AprFeeConfig {
            surcharge_bps: u32::MAX,
        });
        assert_eq!(
            validate_late_fee_config(&max_u32_bps).unwrap_err(),
            ContractError::RateTooHigh
        );
        assert_eq!(
            set_late_fee(&mut deps, &admin_addr, Some(max_u32_bps)).unwrap_err(),
            ContractError::RateTooHigh
        );
    }
}

mod state_transition_and_clearing {
    use super::*;

    #[test]
    fn alternating_fee_modes_leaves_no_stale_state() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let admin_addr = admin(&deps);

        assert_eq!(query_late_fee(&deps), None);
        assert_eq!(
            compute_late_fee_optional(query_late_fee(&deps), 10).unwrap(),
            Uint128::zero()
        );

        let flat_1 = LateFeeConfig::Flat(FlatFeeConfig {
            amount: Uint128::new(150),
        });
        set_late_fee(&mut deps, &admin_addr, Some(flat_1)).unwrap();
        assert_eq!(query_late_fee(&deps), Some(flat_1));
        assert_eq!(
            compute_late_fee_optional(query_late_fee(&deps), 3).unwrap(),
            Uint128::new(450)
        );

        let apr_1 = LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 400 });
        set_late_fee(&mut deps, &admin_addr, Some(apr_1)).unwrap();
        assert_eq!(query_late_fee(&deps), Some(apr_1));
        assert_eq!(
            compute_late_fee_optional(query_late_fee(&deps), 3).unwrap(),
            Uint128::zero()
        );

        set_late_fee(&mut deps, &admin_addr, None).unwrap();
        assert_eq!(query_late_fee(&deps), None);
        assert_eq!(
            compute_late_fee_optional(query_late_fee(&deps), 10).unwrap(),
            Uint128::zero()
        );

        let flat_2 = LateFeeConfig::Flat(FlatFeeConfig {
            amount: Uint128::new(999),
        });
        set_late_fee(&mut deps, &admin_addr, Some(flat_2)).unwrap();
        assert_eq!(query_late_fee(&deps), Some(flat_2));
        assert_eq!(
            compute_late_fee_optional(query_late_fee(&deps), 2).unwrap(),
            Uint128::new(1998)
        );

        let apr_2 = LateFeeConfig::AprBased(AprFeeConfig {
            surcharge_bps: 10_000,
        });
        set_late_fee(&mut deps, &admin_addr, Some(apr_2)).unwrap();
        assert_eq!(query_late_fee(&deps), Some(apr_2));

        set_late_fee(&mut deps, &admin_addr, None).unwrap();
        assert_eq!(query_late_fee(&deps), None);
    }

    #[test]
    fn raw_storage_key_isolation_across_transitions() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let admin_addr = admin(&deps);

        assert!(deps.storage.get(b"lfc").is_none());

        let flat = LateFeeConfig::Flat(FlatFeeConfig {
            amount: Uint128::new(200),
        });
        set_late_fee(&mut deps, &admin_addr, Some(flat)).unwrap();
        let raw_flat = deps.storage.get(b"lfc").expect("key must exist");
        let decoded_flat: LateFeeConfig = from_json(&raw_flat).unwrap();
        assert_eq!(decoded_flat, flat);

        let apr = LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 850 });
        set_late_fee(&mut deps, &admin_addr, Some(apr)).unwrap();
        let raw_apr = deps.storage.get(b"lfc").expect("key must exist");
        let decoded_apr: LateFeeConfig = from_json(&raw_apr).unwrap();
        assert_eq!(decoded_apr, apr);

        set_late_fee(&mut deps, &admin_addr, None).unwrap();
        assert!(deps.storage.get(b"lfc").is_none());
    }
}

mod golden_tests {
    use super::*;

    struct FlatGoldenVector {
        amount: u128,
        installments: u64,
        expected: u128,
    }

    #[test]
    fn golden_tests_flat_variants() {
        let vectors = [
            FlatGoldenVector {
                amount: 1,
                installments: 0,
                expected: 0,
            },
            FlatGoldenVector {
                amount: 1,
                installments: 1,
                expected: 1,
            },
            FlatGoldenVector {
                amount: 10,
                installments: 7,
                expected: 70,
            },
            FlatGoldenVector {
                amount: 50,
                installments: 12,
                expected: 600,
            },
            FlatGoldenVector {
                amount: 250,
                installments: 4,
                expected: 1000,
            },
            FlatGoldenVector {
                amount: 100_000,
                installments: 52,
                expected: 5_200_000,
            },
            FlatGoldenVector {
                amount: 1_000_000_000,
                installments: 365,
                expected: 365_000_000_000,
            },
        ];

        for v in vectors {
            let config = LateFeeConfig::Flat(FlatFeeConfig {
                amount: Uint128::new(v.amount),
            });
            let actual = compute_late_fee(config, v.installments).unwrap();
            assert_eq!(actual, Uint128::new(v.expected));
        }
    }

    struct AprPenaltyGoldenVector {
        principal: u128,
        surcharge_bps: u32,
        elapsed_seconds: u64,
        expected_penalty: u128,
    }

    #[test]
    fn golden_tests_apr_variants_with_floor_rounding() {
        let vectors = [
            AprPenaltyGoldenVector {
                principal: 10_000,
                surcharge_bps: 500,
                elapsed_seconds: creditra_credit::accrual::SECONDS_PER_YEAR,
                expected_penalty: 500,
            },
            AprPenaltyGoldenVector {
                principal: 1_000_000,
                surcharge_bps: 250,
                elapsed_seconds: creditra_credit::accrual::SECONDS_PER_YEAR,
                expected_penalty: 25_000,
            },
            AprPenaltyGoldenVector {
                principal: 100_000,
                surcharge_bps: 1_000,
                elapsed_seconds: creditra_credit::accrual::SECONDS_PER_YEAR / 2,
                expected_penalty: 5_000,
            },
            AprPenaltyGoldenVector {
                principal: 50_000_000,
                surcharge_bps: 300,
                elapsed_seconds: creditra_credit::accrual::SECONDS_PER_YEAR / 4,
                expected_penalty: 375_000,
            },
            AprPenaltyGoldenVector {
                principal: 10_000,
                surcharge_bps: 100,
                elapsed_seconds: 100,
                expected_penalty: 0,
            },
        ];

        for v in vectors {
            let config = AprFeeConfig {
                surcharge_bps: v.surcharge_bps,
            };
            let actual =
                compute_apr_late_fee(Uint128::new(v.principal), &config, v.elapsed_seconds)
                    .unwrap();
            assert_eq!(actual, Uint128::new(v.expected_penalty));

            let public_cfg = LateFeeConfig::AprBased(config);
            assert_eq!(compute_late_fee(public_cfg, 10).unwrap(), Uint128::zero());
        }
    }

    #[test]
    fn golden_tests_cleared_variant() {
        let test_installments = [0, 1, 2, 5, 12, 52, 365, u64::MAX];
        for installments in test_installments {
            assert_eq!(
                compute_late_fee_optional(None, installments).unwrap(),
                Uint128::zero()
            );
        }
    }

    #[test]
    fn golden_tests_boundary_vectors() {
        let max_flat = LateFeeConfig::Flat(FlatFeeConfig {
            amount: Uint128::MAX,
        });
        assert_eq!(compute_late_fee(max_flat, 0).unwrap(), Uint128::zero());
        assert_eq!(compute_late_fee(max_flat, 1).unwrap(), Uint128::MAX);

        let unit_flat = LateFeeConfig::Flat(FlatFeeConfig {
            amount: Uint128::new(1),
        });
        assert_eq!(
            compute_late_fee(unit_flat, u64::MAX).unwrap(),
            Uint128::from(u64::MAX)
        );
    }
}

mod authorization_boundaries {
    use super::*;

    #[test]
    fn unauthorized_caller_cannot_set_or_clear_late_fee_config() {
        let mut deps = mock_dependencies();
        setup(&mut deps);
        let non_admin = attacker(&deps);

        let flat = LateFeeConfig::Flat(FlatFeeConfig {
            amount: Uint128::new(100),
        });
        let apr = LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 200 });

        let err1 = set_late_fee(&mut deps, &non_admin, Some(flat)).unwrap_err();
        assert_eq!(err1, ContractError::Unauthorized);

        let err2 = set_late_fee(&mut deps, &non_admin, Some(apr)).unwrap_err();
        assert_eq!(err2, ContractError::Unauthorized);

        let err3 = set_late_fee(&mut deps, &non_admin, None).unwrap_err();
        assert_eq!(err3, ContractError::Unauthorized);
    }
}
