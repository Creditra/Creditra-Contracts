// SPDX-License-Identifier: MIT

//! Seeded invariant tests for the global `total_utilized` accumulator.
//!
//! The invariant under test is simple but security-critical:
//!
//! `stored_total_utilized == sum(enumerate_credit_lines().utilized_amount)`
//!
//! We drive randomized-but-deterministic sequences across multiple borrowers and
//! re-check the invariant after every successful operation.
//!
//! Covered paths:
//! - `draw_credit`
//! - `repay_credit`
//! - `forgive_debt`
//! - `default_credit_line`
//! - `close_credit_line`
//! - re-opening previously non-active lines to keep the sequence moving

use creditra_credit::types::CreditStatus;
use creditra_credit::{Credit, CreditClient};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{vec, Address, Env, Vec};

const PAGE_SIZE: u32 = 2;
const BORROWER_COUNT: usize = 5;
const STEPS_PER_SEED: usize = 80;
const SEEDS: [u64; 4] = [7, 42, 1_337, 20_240_527];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct CoverageCounters {
    draws: u32,
    repays: u32,
    forgives: u32,
    defaults: u32,
    closes: u32,
    reopens: u32,
    suspends: u32,
}

impl CoverageCounters {
    fn add_assign(&mut self, other: Self) {
        self.draws += other.draws;
        self.repays += other.repays;
        self.forgives += other.forgives;
        self.defaults += other.defaults;
        self.closes += other.closes;
        self.reopens += other.reopens;
        self.suspends += other.suspends;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operation {
    Draw,
    Repay,
    Forgive,
    Default,
    Close,
    Reopen,
    Suspend,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StepRecord {
    borrower_index: usize,
    op: Operation,
    amount: i128,
}

struct Lcg64 {
    state: u64,
}

impl Lcg64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.state
    }

    fn index(&mut self, upper_exclusive: usize) -> usize {
        (self.next_u64() as usize) % upper_exclusive
    }

    fn range_i128(&mut self, inclusive_max: i128) -> i128 {
        1 + (self.next_u64() as i128 % inclusive_max.max(1))
    }

    fn range_u64(&mut self, inclusive_max: u64) -> u64 {
        1 + (self.next_u64() % inclusive_max.max(1))
    }
}

fn setup_env() -> (Env, CreditClient<'static>, Address, Vec<Address>) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(&env, &contract_id);
    client.init(&admin);

    let token_id = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token = token_id.address();
    client.set_liquidity_token(&token);
    client.set_liquidity_source(&contract_id);

    let sac = StellarAssetClient::new(&env, &token);
    sac.mint(&contract_id, &50_000_000_i128);

    let borrowers: Vec<Address> = vec![
        &env,
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
    ];

    for index in 0..BORROWER_COUNT {
        let borrower = borrowers.get(index as u32).unwrap();
        sac.mint(&borrower, &10_000_000_i128);

        let credit_limit = 75_000_i128 + (index as i128 * 15_000_i128);
        let interest_rate_bps = 1_500_u32 + (index as u32 * 900_u32);
        let risk_score = 35_u32 + (index as u32 * 10_u32);
        client.open_credit_line(&borrower, &credit_limit, &interest_rate_bps, &risk_score);
        assert_total_utilized_invariant(&client);
    }

    (env, client, admin, borrowers)
}

fn assert_total_utilized_invariant(client: &CreditClient<'_>) {
    let mut cursor = None;
    let mut enumerated = 0_u32;
    let mut recomputed_total = 0_i128;
    let expected_count = client.get_credit_line_count();

    loop {
        let page = client.enumerate_credit_lines(&cursor, &PAGE_SIZE);
        if page.is_empty() {
            break;
        }

        for item in page.iter() {
            let (id, line) = item;
            enumerated += 1;
            recomputed_total += line.utilized_amount;
            cursor = Some(id);
        }
    }

    assert_eq!(
        enumerated, expected_count,
        "enumeration count mismatch: enumerated={enumerated}, stored={expected_count}"
    );

    let stored_total = client.get_total_utilized();
    assert_eq!(
        stored_total, recomputed_total,
        "total_utilized mismatch: stored={stored_total}, recomputed={recomputed_total}"
    );
}

fn valid_operations(status: CreditStatus, utilized_amount: i128) -> std::vec::Vec<Operation> {
    let mut ops = std::vec![Operation::Close];

    match status {
        CreditStatus::Active => {
            ops.push(Operation::Draw);
            ops.push(Operation::Default);
            ops.push(Operation::Suspend);
        }
        CreditStatus::Suspended => {
            ops.push(Operation::Default);
            ops.push(Operation::Reopen);
        }
        CreditStatus::Defaulted => {
            ops.push(Operation::Reopen);
        }
        CreditStatus::Closed => {
            ops.push(Operation::Reopen);
        }
        CreditStatus::Restricted => {
            ops.push(Operation::Reopen);
        }
    }

    if status != CreditStatus::Closed && utilized_amount > 0 {
        ops.push(Operation::Repay);
        ops.push(Operation::Forgive);
    }

    ops
}

fn apply_operation(
    env: &Env,
    client: &CreditClient<'_>,
    admin: &Address,
    borrower: &Address,
    borrower_index: usize,
    rng: &mut Lcg64,
    counters: &mut CoverageCounters,
) -> StepRecord {
    env.ledger()
        .with_mut(|ledger| ledger.timestamp += rng.range_u64(120 * 24 * 60 * 60));

    let line = client
        .get_credit_line(borrower)
        .expect("credit line exists");
    let ops = valid_operations(line.status, line.utilized_amount);
    let chosen = ops[rng.index(ops.len())];

    let record = match chosen {
        Operation::Draw => {
            let remaining = (line.credit_limit - line.utilized_amount).max(1);
            let amount = rng.range_i128(remaining.min(12_500));
            client.draw_credit(borrower, &amount);
            counters.draws += 1;
            StepRecord {
                borrower_index,
                op: Operation::Draw,
                amount,
            }
        }
        Operation::Repay => {
            let amount = rng.range_i128((line.utilized_amount + 2_500).max(1));
            client.repay_credit(borrower, &amount);
            counters.repays += 1;
            StepRecord {
                borrower_index,
                op: Operation::Repay,
                amount,
            }
        }
        Operation::Forgive => {
            let amount = rng.range_i128((line.utilized_amount + 2_500).max(1));
            client.forgive_debt(borrower, &amount);
            counters.forgives += 1;
            StepRecord {
                borrower_index,
                op: Operation::Forgive,
                amount,
            }
        }
        Operation::Default => {
            client.default_credit_line(borrower);
            counters.defaults += 1;
            StepRecord {
                borrower_index,
                op: Operation::Default,
                amount: 0,
            }
        }
        Operation::Close => {
            client.close_credit_line(borrower, admin);
            counters.closes += 1;
            StepRecord {
                borrower_index,
                op: Operation::Close,
                amount: 0,
            }
        }
        Operation::Reopen => {
            let new_limit = 80_000_i128 + (borrower_index as i128 * 20_000_i128);
            let new_rate = 2_000_u32 + (borrower_index as u32 * 500_u32);
            let new_score = 40_u32 + (borrower_index as u32 * 8_u32);
            client.open_credit_line(borrower, &new_limit, &new_rate, &new_score);
            counters.reopens += 1;
            StepRecord {
                borrower_index,
                op: Operation::Reopen,
                amount: 0,
            }
        }
        Operation::Suspend => {
            client.suspend_credit_line(borrower);
            counters.suspends += 1;
            StepRecord {
                borrower_index,
                op: Operation::Suspend,
                amount: 0,
            }
        }
    };

    assert_total_utilized_invariant(client);
    record
}

fn run_seed(seed: u64) -> (CoverageCounters, std::vec::Vec<StepRecord>) {
    let (env, client, admin, borrowers) = setup_env();
    let mut rng = Lcg64::new(seed);
    let mut counters = CoverageCounters::default();
    let mut trace = std::vec::Vec::with_capacity(STEPS_PER_SEED);

    assert_total_utilized_invariant(&client);

    for _ in 0..STEPS_PER_SEED {
        let borrower_index = rng.index(BORROWER_COUNT);
        let borrower = borrowers.get(borrower_index as u32).unwrap();
        let step = apply_operation(
            &env,
            &client,
            &admin,
            &borrower,
            borrower_index,
            &mut rng,
            &mut counters,
        );
        trace.push(step);
    }

    assert_total_utilized_invariant(&client);
    (counters, trace)
}

#[test]
fn total_utilized_invariant_holds_after_every_seeded_operation() {
    let mut aggregate = CoverageCounters::default();

    for seed in SEEDS {
        let (per_seed, _trace) = run_seed(seed);
        aggregate.add_assign(per_seed);
    }

    assert!(aggregate.draws > 0, "draw path was not covered");
    assert!(aggregate.repays > 0, "repay path was not covered");
    assert!(aggregate.forgives > 0, "forgive path was not covered");
    assert!(aggregate.defaults > 0, "default path was not covered");
    assert!(aggregate.closes > 0, "close path was not covered");
}

#[test]
fn total_utilized_randomized_trace_is_deterministic_for_fixed_seed() {
    let (counters_a, trace_a) = run_seed(42);
    let (counters_b, trace_b) = run_seed(42);

    assert_eq!(
        counters_a, counters_b,
        "coverage counters diverged for same seed"
    );
    assert_eq!(trace_a, trace_b, "operation trace diverged for same seed");
}
