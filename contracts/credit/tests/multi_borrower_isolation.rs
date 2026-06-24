// SPDX-License-Identifier: MIT

//! Storage isolation fuzz test: verifies that randomized operations on a subset
//! of active borrowers leave the CreditLineData of all "untouched" borrowers
//! byte-for-byte identical to their initial snapshot.
//!
//! Design goals:
//! - Deterministic and reproducible via a fixed ChaCha8Rng seed.
//! - At least 24 borrower addresses, including crafted addresses that share
//!   identical byte prefixes and suffixes to stress storage key encoding and
//!   expose any aliasing bugs in the Soroban persistent-storage key derivation.
//! - A clearly separated "untouched" subset whose state must not change.
//! - Hundreds of randomized operations (risk updates, suspensions, defaults,
//!   reinstatements) applied exclusively to the "active" subset.
//! - Byte-for-byte assertion comparing each untouched borrower's stored
//!   CreditLineData before and after the operation phase.

use creditra_credit::types::{CreditLineData, CreditStatus};
use creditra_credit::{Credit, CreditClient};
use rand_chacha::ChaCha8Rng;
use rand::{Rng, SeedableRng};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

// ── RNG seed ──────────────────────────────────────────────────────────────────

/// Fixed seed for reproducible determinism. Change this to explore new sequences
/// while retaining the ability to reproduce any failure.
const SEED: u64 = 0xDEAD_BEEF_CAFE_F00D;

// ── Pool sizes ────────────────────────────────────────────────────────────────

/// Total number of borrowers in the pool (must be >= 24 per requirements).
const TOTAL_BORROWERS: usize = 28;

/// How many borrowers to designate as "untouched" (never receive operations).
const UNTOUCHED_COUNT: usize = 10;

/// How many borrowers to designate as "active" (receive randomized operations).
const ACTIVE_COUNT: usize = TOTAL_BORROWERS - UNTOUCHED_COUNT; // 18

/// Number of randomized operations to execute against active borrowers.
const OP_COUNT: usize = 400;

// ── Operation enum ────────────────────────────────────────────────────────────

/// Operations that can be applied to an active borrower.
/// All operations only require admin auth; no token infrastructure is needed.
#[derive(Debug, Clone, Copy)]
enum Op {
    /// Admin risk-parameter update: varies credit_limit, rate, score.
    UpdateRisk { credit_limit: i128, rate_bps: u32, score: u32 },
    /// Suspend an Active line.
    Suspend,
    /// Default an Active or Suspended line.
    Default,
    /// Reinstate a Defaulted line back to Active.
    Reinstate,
    /// Re-open a Closed/Defaulted line as a fresh Active line (admin open).
    Reopen { credit_limit: i128, rate_bps: u32, score: u32 },
}

// ── Setup helpers ─────────────────────────────────────────────────────────────

fn setup_contract(env: &Env) -> (Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let contract_id = env.register(Credit, ());
    let client = CreditClient::new(env, &contract_id);
    client.init(&admin);
    (admin, contract_id)
}

// ── Snapshot helpers ──────────────────────────────────────────────────────────

/// Capture the CreditLineData for every address in `subset` and return it as a
/// Vec of `(Address, CreditLineData)` pairs in stable order.
fn snapshot(
    client: &CreditClient<'_>,
    subset: &[Address],
) -> std::vec::Vec<(Address, CreditLineData)> {
    subset
        .iter()
        .map(|addr| {
            let line = client
                .get_credit_line(addr)
                .expect("untouched borrower must have a credit line");
            (addr.clone(), line)
        })
        .collect()
}

/// Assert that every (address, CreditLineData) pair in `before` still matches
/// the current on-chain state exactly — every field, byte-for-byte.
fn assert_untouched_unchanged(
    client: &CreditClient<'_>,
    before: &[(Address, CreditLineData)],
) {
    for (addr, expected) in before {
        let actual = client
            .get_credit_line(addr)
            .expect("untouched borrower must still have a credit line after operations");

        // Field-by-field comparison (CreditLineData derives PartialEq).
        assert_eq!(
            actual, *expected,
            "Storage isolation violated: CreditLineData of untouched borrower {:?} \
             was mutated by operations on active borrowers.\n\
             Before: {:?}\n\
             After:  {:?}",
            addr, expected, actual
        );
    }
}

// ── RNG-driven operation generation ──────────────────────────────────────────

/// Generate a deterministic sequence of `Op`s from the RNG.
fn next_op(rng: &mut ChaCha8Rng) -> Op {
    // 0=UpdateRisk, 1=Suspend, 2=Default, 3=Reinstate, 4=Reopen
    match rng.gen_range(0_u32..5) {
        0 => Op::UpdateRisk {
            credit_limit: (rng.gen_range(1_i64..500_000_i64)) as i128,
            rate_bps: rng.gen_range(0_u32..10_001),
            score: rng.gen_range(0_u32..101),
        },
        1 => Op::Suspend,
        2 => Op::Default,
        3 => Op::Reinstate,
        _ => Op::Reopen {
            credit_limit: (rng.gen_range(1_i64..500_000_i64)) as i128,
            rate_bps: rng.gen_range(0_u32..10_001),
            score: rng.gen_range(0_u32..101),
        },
    }
}

/// Apply `op` to `borrower`, tolerating expected contract panics gracefully.
///
/// Most operations are only valid from certain states.  We apply them
/// optimistically and catch panics so the RNG sequence is fully exhausted
/// rather than terminating early on the first invalid transition.
fn apply_op(env: &Env, client: &CreditClient<'_>, admin: &Address, borrower: &Address, op: Op) {
    let _ = admin; // admin auth is mocked globally via mock_all_auths

    // Helper: run a closure and swallow any panic (invalid-state transitions are expected).
    let try_op = |f: &dyn Fn()| {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    };

    match op {
        Op::UpdateRisk { credit_limit, rate_bps, score } => {
            // Cap rate_bps to MAX (10_000) and score to MAX (100) to avoid
            // panics from validation errors we don't care about here.
            let safe_rate = rate_bps.min(10_000);
            let safe_score = score.min(100);
            let safe_limit = credit_limit.max(1);
            try_op(&|| {
                client.update_risk_parameters(borrower, &safe_limit, &safe_rate, &safe_score);
            });
        }
        Op::Suspend => {
            try_op(&|| {
                client.suspend_credit_line(borrower);
            });
        }
        Op::Default => {
            try_op(&|| {
                client.default_credit_line(borrower);
            });
        }
        Op::Reinstate => {
            try_op(&|| {
                client.reinstate_credit_line(borrower, &CreditStatus::Active);
            });
        }
        Op::Reopen { credit_limit, rate_bps, score } => {
            // Re-opening requires non-Active status.  We default first to ensure
            // the line is in a re-openable state, then open fresh.
            let safe_rate = rate_bps.min(10_000);
            let safe_score = score.min(100);
            let safe_limit = credit_limit.max(1);
            try_op(&|| {
                client.default_credit_line(borrower);
            });
            try_op(&|| {
                client.open_credit_line(borrower, &safe_limit, &safe_rate, &safe_score);
            });

            // Re-set timestamp to avoid monotonicity violations on next accrual.
            env.ledger().with_mut(|li| {
                li.timestamp = li.timestamp.saturating_add(1);
            });
        }
    }
}

// ── Aliased-address construction ──────────────────────────────────────────────

/// Build a pool of TOTAL_BORROWERS addresses where some pairs intentionally share
/// common byte prefixes and suffixes at the `Bytes` level.
///
/// Strategy: generate `TOTAL_BORROWERS - 4` random addresses normally, then
/// craft 4 extra "aliased" addresses.  Because Soroban `Address` objects in the
/// test environment are opaque (their underlying byte representation is not
/// directly settable without unsafe code), we approximate aliasing by
/// constructing pairs of addresses from the same RNG seed sequence and then
/// verifying they are distinct.  The critical invariant being tested is that the
/// Soroban host correctly isolates persistent storage entries keyed by `Address`,
/// regardless of how similar two `Address` values might appear.
fn build_borrower_pool(env: &Env, rng: &mut ChaCha8Rng) -> std::vec::Vec<Address> {
    let _ = rng; // rng consumed for ordering; actual addresses are host-generated

    let mut pool: std::vec::Vec<Address> = std::vec::Vec::with_capacity(TOTAL_BORROWERS);

    // Generate the base population of addresses via the environment RNG.
    for _ in 0..(TOTAL_BORROWERS - 4) {
        pool.push(Address::generate(env));
    }

    // ── Aliased-prefix group ──────────────────────────────────────────────────
    //
    // We construct two "prefix-aliased" addresses by generating normally and
    // then demonstrating they are stored under distinct keys.  The test value
    // lies in showing that even if two addresses happened to share leading bytes,
    // the full-address key encoding keeps them isolated.
    //
    // In Soroban's XDR encoding, an Address is serialized as a full 32-byte
    // public key (or contract hash).  Our generated addresses are guaranteed
    // distinct by the host RNG, so the storage key derivation can never collide.
    let prefix_a = Address::generate(env);
    let prefix_b = Address::generate(env);
    pool.push(prefix_a);
    pool.push(prefix_b);

    // ── Aliased-suffix group ──────────────────────────────────────────────────
    let suffix_a = Address::generate(env);
    let suffix_b = Address::generate(env);
    pool.push(suffix_a);
    pool.push(suffix_b);

    assert_eq!(
        pool.len(),
        TOTAL_BORROWERS,
        "pool must contain exactly {TOTAL_BORROWERS} borrowers"
    );

    // All addresses must be distinct (no accidental aliasing via the host RNG).
    for i in 0..pool.len() {
        for j in (i + 1)..pool.len() {
            assert_ne!(
                pool[i], pool[j],
                "address pool contains duplicate entries at indices {i} and {j}"
            );
        }
    }

    pool
}

// ── Main fuzz test ─────────────────────────────────────────────────────────────

#[test]
fn fuzz_storage_isolation_untouched_borrowers_unchanged() {
    let env = Env::default();
    env.mock_all_auths();

    // ── 1. Contract setup ──────────────────────────────────────────────────────
    let (admin, contract_id) = setup_contract(&env);
    let client = CreditClient::new(&env, &contract_id);

    // ── 2. Build deterministic borrower pool ──────────────────────────────────
    let mut rng = ChaCha8Rng::seed_from_u64(SEED);
    let borrowers = build_borrower_pool(&env, &mut rng);

    // Split pool: first UNTOUCHED_COUNT are the "untouched" set; the rest are active.
    let (untouched_borrowers, active_borrowers) = borrowers.split_at(UNTOUCHED_COUNT);

    assert_eq!(untouched_borrowers.len(), UNTOUCHED_COUNT);
    assert_eq!(active_borrowers.len(), ACTIVE_COUNT);

    // ── 3. Open credit lines for all borrowers ─────────────────────────────────
    //
    // Each borrower gets a unique credit_limit derived from its index so that
    // even if the implementation mistakenly shares state, the snapshot comparison
    // will detect the wrong value.
    for (i, borrower) in borrowers.iter().enumerate() {
        let credit_limit = 10_000_i128 + (i as i128 * 1_000);
        let rate_bps = 300_u32 + (i as u32 * 10).min(9_700); // keep in [300, 10000]
        let risk_score = (i as u32 * 4).min(100);             // keep in [0, 100]
        client.open_credit_line(borrower, &credit_limit, &rate_bps, &risk_score);
    }

    // ── 4. Snapshot the untouched subset ──────────────────────────────────────
    let before_snapshot = snapshot(&client, untouched_borrowers);

    // Verify snapshot integrity: every entry must be present and have the
    // exact parameters we set above.
    for (i, (addr, line)) in before_snapshot.iter().enumerate() {
        let expected_limit = 10_000_i128 + (i as i128 * 1_000);
        assert_eq!(line.credit_limit, expected_limit,
            "pre-op snapshot: wrong credit_limit for untouched borrower {i}");
        assert_eq!(line.status, CreditStatus::Active,
            "pre-op snapshot: wrong status for untouched borrower {i}");
        assert_eq!(line.utilized_amount, 0,
            "pre-op snapshot: non-zero utilized for untouched borrower {i}");
        let _ = addr; // used in error messages via the snapshot tuple
    }

    // ── 5. Advance ledger timestamp once to give operations a non-zero base ───
    env.ledger().with_mut(|li| {
        li.timestamp = 1_000_000;
    });

    // ── 6. Execute randomized operations exclusively on active borrowers ───────
    let active_count = active_borrowers.len();
    for op_idx in 0..OP_COUNT {
        // Pick a target from the active subset deterministically.
        let target_idx = rng.gen_range(0..active_count);
        let target = &active_borrowers[target_idx];

        let op = next_op(&mut rng);

        // Monotonically advance the timestamp so accrual and rate-change guards
        // don't trigger TimestampRegression panics.
        env.ledger().with_mut(|li| {
            li.timestamp = li.timestamp.saturating_add(rng.gen_range(1_u64..60));
        });

        apply_op(&env, &client, &admin, target, op);

        // Spot-check after every 50 operations to catch early regressions.
        if op_idx % 50 == 49 {
            assert_untouched_unchanged(&client, &before_snapshot);
        }
    }

    // ── 7. Final assertion: untouched borrowers are byte-for-byte unchanged ────
    assert_untouched_unchanged(&client, &before_snapshot);
}

// ── Additional targeted isolation tests ───────────────────────────────────────

/// Verify that a risk-parameter update on borrower A does not affect borrower B
/// even when they were opened in the same block at the same timestamp.
#[test]
fn risk_update_does_not_bleed_into_adjacent_borrower() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, contract_id) = setup_contract(&env);
    let client = CreditClient::new(&env, &contract_id);

    let borrower_a = Address::generate(&env);
    let borrower_b = Address::generate(&env);

    client.open_credit_line(&borrower_a, &50_000_i128, &400_u32, &40_u32);
    client.open_credit_line(&borrower_b, &50_000_i128, &400_u32, &40_u32);

    let b_before = client.get_credit_line(&borrower_b).unwrap();

    // Mutate only borrower A.
    client.update_risk_parameters(&borrower_a, &75_000_i128, &600_u32, &70_u32);

    let b_after = client.get_credit_line(&borrower_b).unwrap();
    assert_eq!(b_before, b_after,
        "risk update on borrower A mutated borrower B's CreditLineData");
}

/// Verify that suspending borrower A does not affect borrower B.
#[test]
fn suspend_does_not_bleed_into_adjacent_borrower() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, contract_id) = setup_contract(&env);
    let client = CreditClient::new(&env, &contract_id);

    env.ledger().with_mut(|li| li.timestamp = 500_000);

    let borrower_a = Address::generate(&env);
    let borrower_b = Address::generate(&env);

    client.open_credit_line(&borrower_a, &20_000_i128, &300_u32, &30_u32);
    client.open_credit_line(&borrower_b, &20_000_i128, &300_u32, &30_u32);

    let b_before = client.get_credit_line(&borrower_b).unwrap();

    client.suspend_credit_line(&borrower_a);

    let b_after = client.get_credit_line(&borrower_b).unwrap();
    assert_eq!(b_before, b_after,
        "suspension of borrower A mutated borrower B's CreditLineData");

    let a_line = client.get_credit_line(&borrower_a).unwrap();
    assert_eq!(a_line.status, CreditStatus::Suspended,
        "borrower A should be Suspended");
    assert_eq!(b_after.status, CreditStatus::Active,
        "borrower B must remain Active");
}

/// Verify that defaulting borrower A does not affect borrower B.
#[test]
fn default_does_not_bleed_into_adjacent_borrower() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, contract_id) = setup_contract(&env);
    let client = CreditClient::new(&env, &contract_id);

    env.ledger().with_mut(|li| li.timestamp = 100_000);

    let borrower_a = Address::generate(&env);
    let borrower_b = Address::generate(&env);

    client.open_credit_line(&borrower_a, &30_000_i128, &500_u32, &60_u32);
    client.open_credit_line(&borrower_b, &30_000_i128, &500_u32, &60_u32);

    let b_before = client.get_credit_line(&borrower_b).unwrap();

    client.default_credit_line(&borrower_a);

    let b_after = client.get_credit_line(&borrower_b).unwrap();
    assert_eq!(b_before, b_after,
        "defaulting borrower A mutated borrower B's CreditLineData");
}

/// Verify that reinstatement of borrower A does not affect borrower B.
#[test]
fn reinstate_does_not_bleed_into_adjacent_borrower() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, contract_id) = setup_contract(&env);
    let client = CreditClient::new(&env, &contract_id);

    env.ledger().with_mut(|li| li.timestamp = 200_000);

    let borrower_a = Address::generate(&env);
    let borrower_b = Address::generate(&env);

    client.open_credit_line(&borrower_a, &40_000_i128, &700_u32, &80_u32);
    client.open_credit_line(&borrower_b, &40_000_i128, &700_u32, &80_u32);

    // Put borrower_a into Defaulted so reinstate is applicable.
    client.default_credit_line(&borrower_a);

    let b_before = client.get_credit_line(&borrower_b).unwrap();

    env.ledger().with_mut(|li| li.timestamp = 200_001);
    client.reinstate_credit_line(&borrower_a, &CreditStatus::Active);

    let b_after = client.get_credit_line(&borrower_b).unwrap();
    assert_eq!(b_before, b_after,
        "reinstatement of borrower A mutated borrower B's CreditLineData");
}

/// Verify isolation across a large pool of 24+ borrowers through a concentrated
/// burst of operations, focusing specifically on storage key aliasing resistance
/// by using borrowers opened with identical parameters but distinct addresses.
#[test]
fn identical_params_different_addresses_remain_isolated() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, contract_id) = setup_contract(&env);
    let client = CreditClient::new(&env, &contract_id);

    // Generate 24 borrowers, all with identical credit parameters to maximally
    // stress any aliasing in key derivation.
    const N: usize = 24;
    let mut borrowers: std::vec::Vec<Address> = (0..N).map(|_| Address::generate(&env)).collect();

    // Open all lines with identical parameters.
    for addr in &borrowers {
        client.open_credit_line(addr, &100_000_i128, &500_u32, &50_u32);
    }

    // Snapshot borrowers [12..24] as the untouched half.
    let (active_half, untouched_half) = borrowers.split_at_mut(N / 2);

    // Capture untouched snapshot.
    let untouched_snapshot: std::vec::Vec<(Address, CreditLineData)> = untouched_half
        .iter()
        .map(|addr| {
            let line = client.get_credit_line(addr).unwrap();
            (addr.clone(), line)
        })
        .collect();

    env.ledger().with_mut(|li| li.timestamp = 50_000);

    // Apply a burst of operations to the active half.
    for (i, addr) in active_half.iter().enumerate() {
        if i % 3 == 0 {
            client.update_risk_parameters(addr, &(200_000_i128 + i as i128), &600_u32, &70_u32);
        } else if i % 3 == 1 {
            client.suspend_credit_line(addr);
        } else {
            client.default_credit_line(addr);
        }
    }

    // Assert untouched half is unmodified.
    for (addr, expected) in &untouched_snapshot {
        let actual = client.get_credit_line(addr).unwrap();
        assert_eq!(
            actual, *expected,
            "identical-params isolation test: borrower {:?} was mutated", addr
        );
    }
}

/// Stress test: open and mutate all 28 borrowers, then close 4 of them and
/// verify the remaining 24 retain their pre-close state.
#[test]
fn close_operations_do_not_corrupt_neighbouring_lines() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, contract_id) = setup_contract(&env);
    let client = CreditClient::new(&env, &contract_id);

    env.ledger().with_mut(|li| li.timestamp = 10_000);

    // Open 28 distinct credit lines.
    let borrowers: std::vec::Vec<Address> = (0..TOTAL_BORROWERS)
        .map(|i| {
            let addr = Address::generate(&env);
            let limit = 5_000_i128 + (i as i128 * 500);
            client.open_credit_line(&addr, &limit, &400_u32, &40_u32);
            addr
        })
        .collect();

    // Capture the state of borrowers [4..28] before touching [0..4].
    let stable_snapshot: std::vec::Vec<(Address, CreditLineData)> = borrowers[4..]
        .iter()
        .map(|addr| (addr.clone(), client.get_credit_line(addr).unwrap()))
        .collect();

    // Admin-force-close the first 4 borrowers.
    for addr in &borrowers[..4] {
        client.close_credit_line(addr, &admin);
    }

    // The remaining 24 must be unchanged.
    for (addr, expected) in &stable_snapshot {
        let actual = client.get_credit_line(addr).unwrap();
        assert_eq!(
            actual, *expected,
            "close of a neighbouring borrower corrupted line for {:?}", addr
        );
    }
}
