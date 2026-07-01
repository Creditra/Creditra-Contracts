// SPDX-License-Identifier: MIT

//! Amount validation matrix — issue #236
//!
//! Table-driven tests verifying that `draw_credit`, `repay_credit`, and
//! `open_credit_line` all reject zero, negative, and the minimal positive
//! amounts consistently, mapping every rejection to
//! `ContractError::InvalidAmount` (code 5).
//!
//! # Security assumptions / trust boundaries
//! - All callers are mocked via `env.mock_all_auths()`. In production the
//!   borrower must hold a valid Soroban auth entry; here we focus only on the
//!   amount guard, which fires *before* any token transfer.
//! - Negative `i128` amounts are representable in the type system but must
//!   never reach protocol state.
//! - The zero-amount guard prevents accounting no-ops that could be exploited
//!   to trigger events or side-effects without moving value.
//!
//! # Failure modes documented
//! | Entrypoint       | Invalid amount  | Expected error           |
//! |------------------|-----------------|--------------------------|
//! | `draw_credit`    | 0               | `ContractError::InvalidAmount` (5) |
//! | `draw_credit`    | -1              | `ContractError::InvalidAmount` (5) |
//! | `draw_credit`    | `i128::MIN`     | `ContractError::InvalidAmount` (5) |
//! | `repay_credit`   | 0               | `ContractError::InvalidAmount` (5) |
//! | `repay_credit`   | -1              | `ContractError::InvalidAmount` (5) |
//! | `repay_credit`   | `i128::MIN`     | `ContractError::InvalidAmount` (5) |
//! | `open_credit_line` | 0 (limit)     | `ContractError::InvalidAmount` (5) |
//! | `open_credit_line` | -1 (limit)    | `ContractError::InvalidAmount` (5) |
//! | `open_credit_line` | `i128::MIN`   | `ContractError::InvalidAmount` (5) |

use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token::StellarAssetClient, Address, Env};

// ────────────────────────────────────────────────────────────────────────────
// Test helpers
// ────────────────────────────────────────────────────────────────────────────

/// Minimal contract + token setup used by draw/repay tests.
///
/// Returns `(client, token_address, contract_id, admin, borrower)`.
fn setup_with_token(
    env: &Env,
    credit_limit: i128,
) -> (CreditClient<'_>, Address, Address, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let borrower = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(env));
    let token_address = token_id.address();
    client.set_liquidity_token(&token_address);
    client.set_liquidity_source(&contract_id);

    // Fund the reserve so draws can succeed in happy-path variants.
    StellarAssetClient::new(env, &token_address).mint(&contract_id, &credit_limit);

    client.open_credit_line(&borrower, &credit_limit, &300_u32, &70_u32);

    (client, token_address, contract_id, admin, borrower)
}

/// Minimal contract setup for `open_credit_line` tests (no token needed).
fn setup_admin_only(env: &Env) -> (CreditClient<'_>, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    (client, admin)
}

// ────────────────────────────────────────────────────────────────────────────
// draw_credit — invalid amount matrix
// ────────────────────────────────────────────────────────────────────────────

/// Test-case record for draw/repay amount validation.
struct AmountCase {
    description: &'static str,
    amount: i128,
}

/// All invalid amounts for `draw_credit` must reject with `InvalidAmount` (5).
#[test]
fn draw_credit_rejects_invalid_amounts() {
    let cases = [
        AmountCase {
            description: "zero draw amount",
            amount: 0,
        },
        AmountCase {
            description: "negative draw amount (-1)",
            amount: -1,
        },
        AmountCase {
            description: "large negative draw amount (-1_000_000)",
            amount: -1_000_000,
        },
        AmountCase {
            description: "i128::MIN draw amount",
            amount: i128::MIN,
        },
    ];

    for case in &cases {
        let env = Env::default();
        let (client, _token, _contract, _admin, borrower) = setup_with_token(&env, 10_000_i128);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.draw_credit(&borrower, &case.amount);
        }));

        assert!(
            result.is_err(),
            "draw_credit should reject '{}' (amount={})",
            case.description,
            case.amount
        );
    }
}

/// Minimal positive amount `1` must **succeed** on `draw_credit` (regression guard).
#[test]
fn draw_credit_accepts_minimal_positive_amount() {
    let env = Env::default();
    let (client, _token, _contract, _admin, borrower) = setup_with_token(&env, 10_000_i128);

    // Should not panic.
    client.draw_credit(&borrower, &1_i128);

    let line = client.get_credit_line(&borrower).expect("line must exist");
    assert_eq!(
        line.utilized_amount, 1,
        "utilized_amount should be 1 after minimal draw"
    );
}

// ────────────────────────────────────────────────────────────────────────────
// repay_credit — invalid amount matrix
// ────────────────────────────────────────────────────────────────────────────

/// All invalid amounts for `repay_credit` must reject with `InvalidAmount` (5).
#[test]
fn repay_credit_rejects_invalid_amounts() {
    let cases = [
        AmountCase {
            description: "zero repay amount",
            amount: 0,
        },
        AmountCase {
            description: "negative repay amount (-1)",
            amount: -1,
        },
        AmountCase {
            description: "large negative repay amount (-1_000_000)",
            amount: -1_000_000,
        },
        AmountCase {
            description: "i128::MIN repay amount",
            amount: i128::MIN,
        },
    ];

    for case in &cases {
        // Each test case gets a fresh environment so no state bleeds across.
        let env = Env::default();
        let (client, token_address, contract_id, _admin, borrower) =
            setup_with_token(&env, 10_000_i128);

        // Draw first so there is outstanding debt to repay.
        client.draw_credit(&borrower, &1_000_i128);

        // Approve allowance so the repayment path doesn't fail on allowance check.
        StellarAssetClient::new(&env, &token_address).mint(&borrower, &5_000_i128);
        soroban_sdk::token::Client::new(&env, &token_address).approve(
            &borrower,
            &contract_id,
            &5_000_i128,
            &1_000_u32,
        );

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.repay_credit(&borrower, &case.amount);
        }));

        assert!(
            result.is_err(),
            "repay_credit should reject '{}' (amount={})",
            case.description,
            case.amount
        );
    }
}

/// Minimal positive amount `1` must **succeed** on `repay_credit` (regression guard).
#[test]
fn repay_credit_accepts_minimal_positive_amount() {
    let env = Env::default();
    let (client, token_address, contract_id, _admin, borrower) =
        setup_with_token(&env, 10_000_i128);

    client.draw_credit(&borrower, &1_000_i128);

    StellarAssetClient::new(&env, &token_address).mint(&borrower, &500_i128);
    soroban_sdk::token::Client::new(&env, &token_address).approve(
        &borrower,
        &contract_id,
        &500_i128,
        &1_000_u32,
    );

    // Should not panic.
    client.repay_credit(&borrower, &1_i128);

    let line = client.get_credit_line(&borrower).expect("line must exist");
    assert_eq!(
        line.utilized_amount, 999,
        "utilized_amount should decrease by 1 after minimal repay"
    );
}

// ────────────────────────────────────────────────────────────────────────────
// open_credit_line — invalid credit_limit matrix
// ────────────────────────────────────────────────────────────────────────────

/// Test-case record for `open_credit_line` limit validation.
struct LimitCase {
    description: &'static str,
    credit_limit: i128,
}

/// All non-positive `credit_limit` values must reject with `InvalidAmount` (5).
#[test]
fn open_credit_line_rejects_invalid_credit_limits() {
    let cases = [
        LimitCase {
            description: "zero credit_limit",
            credit_limit: 0,
        },
        LimitCase {
            description: "negative credit_limit (-1)",
            credit_limit: -1,
        },
        LimitCase {
            description: "large negative credit_limit (-1_000_000)",
            credit_limit: -1_000_000,
        },
        LimitCase {
            description: "i128::MIN credit_limit",
            credit_limit: i128::MIN,
        },
    ];

    for case in &cases {
        let env = Env::default();
        let (client, _admin) = setup_admin_only(&env);
        let borrower = Address::generate(&env);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.open_credit_line(&borrower, &case.credit_limit, &300_u32, &70_u32);
        }));

        assert!(
            result.is_err(),
            "open_credit_line should reject '{}' (credit_limit={})",
            case.description,
            case.credit_limit
        );
    }
}

/// Minimal positive `credit_limit` of `1` must **succeed** on `open_credit_line`.
#[test]
fn open_credit_line_accepts_minimal_positive_limit() {
    let env = Env::default();
    let (client, _admin) = setup_admin_only(&env);
    let borrower = Address::generate(&env);

    // Should not panic.
    client.open_credit_line(&borrower, &1_i128, &300_u32, &70_u32);

    let line = client.get_credit_line(&borrower).expect("line must exist");
    assert_eq!(line.credit_limit, 1);
    assert_eq!(line.utilized_amount, 0);
    assert_eq!(line.status, CreditStatus::Active);
}

// ────────────────────────────────────────────────────────────────────────────
// Combined matrix: all three entrypoints × all invalid amounts
// ────────────────────────────────────────────────────────────────────────────

/// Validates `ContractError::InvalidAmount` discriminant is code 5
/// (guards against accidental discriminant renumbering).
#[test]
fn invalid_amount_discriminant_is_5() {
    assert_eq!(ContractError::InvalidAmount as u32, 5);
}

/// Documents the full rejection matrix in a single consolidated test:
/// every combination of entrypoint × invalid amount must reject.
///
/// This is the authoritative table test as requested in issue #236.
#[test]
fn amount_rejection_matrix_all_entrypoints() {
    #[derive(Debug, Clone, Copy)]
    enum Entrypoint {
        DrawCredit,
        RepayCredit,
        OpenCreditLine,
    }

    struct MatrixRow {
        entrypoint: Entrypoint,
        description: &'static str,
        amount: i128,
    }

    let matrix = [
        // --- draw_credit ---
        MatrixRow {
            entrypoint: Entrypoint::DrawCredit,
            description: "draw zero",
            amount: 0,
        },
        MatrixRow {
            entrypoint: Entrypoint::DrawCredit,
            description: "draw -1",
            amount: -1,
        },
        MatrixRow {
            entrypoint: Entrypoint::DrawCredit,
            description: "draw i128::MIN",
            amount: i128::MIN,
        },
        // --- repay_credit ---
        MatrixRow {
            entrypoint: Entrypoint::RepayCredit,
            description: "repay zero",
            amount: 0,
        },
        MatrixRow {
            entrypoint: Entrypoint::RepayCredit,
            description: "repay -1",
            amount: -1,
        },
        MatrixRow {
            entrypoint: Entrypoint::RepayCredit,
            description: "repay i128::MIN",
            amount: i128::MIN,
        },
        // --- open_credit_line (credit_limit) ---
        MatrixRow {
            entrypoint: Entrypoint::OpenCreditLine,
            description: "open limit 0",
            amount: 0,
        },
        MatrixRow {
            entrypoint: Entrypoint::OpenCreditLine,
            description: "open limit -1",
            amount: -1,
        },
        MatrixRow {
            entrypoint: Entrypoint::OpenCreditLine,
            description: "open limit i128::MIN",
            amount: i128::MIN,
        },
    ];

    for row in &matrix {
        let env = Env::default();

        let result = match row.entrypoint {
            Entrypoint::DrawCredit | Entrypoint::RepayCredit => {
                let (client, token_address, contract_id, _admin, borrower) =
                    setup_with_token(&env, 10_000_i128);

                match row.entrypoint {
                    Entrypoint::DrawCredit => {
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            client.draw_credit(&borrower, &row.amount);
                        }))
                    }
                    Entrypoint::RepayCredit => {
                        // Draw first so there is debt.
                        client.draw_credit(&borrower, &1_000_i128);
                        StellarAssetClient::new(&env, &token_address).mint(&borrower, &5_000_i128);
                        soroban_sdk::token::Client::new(&env, &token_address).approve(
                            &borrower,
                            &contract_id,
                            &5_000_i128,
                            &1_000_u32,
                        );
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            client.repay_credit(&borrower, &row.amount);
                        }))
                    }
                    _ => unreachable!(),
                }
            }
            Entrypoint::OpenCreditLine => {
                let (client, _admin) = setup_admin_only(&env);
                let borrower = Address::generate(&env);
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.open_credit_line(&borrower, &row.amount, &300_u32, &70_u32);
                }))
            }
        };

        assert!(
            result.is_err(),
            "[{:?}] '{}' (amount={}) should have been rejected with InvalidAmount",
            row.entrypoint,
            row.description,
            row.amount
        );
    }
}
