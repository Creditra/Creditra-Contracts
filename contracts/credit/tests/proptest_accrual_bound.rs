use proptest::prelude::*;
use soroban_sdk::{testutils::Ledger, Env};

// Adjust this import to match your contract's calculation or client location
use creditra_credit::{calculate_accrued_interest, CreditContractClient};

/// NatSpec-style Documentation
/// 
/// # Invariant Test
/// Asserts that for any valid borrower configuration, the accrued interest 
/// never exceeds the total utilized principal amount.
fn check_accrual_invariant(utilized_amount: i128, interest_rate_bps: u32, elapsed_time: u64) -> bool {
    let env = Env::default();
    
    env.ledger().with_mut(|ledger| {
        ledger.timestamp = elapsed_time;
    });

    let accrued_interest = calculate_accrued_interest(&env, utilized_amount, interest_rate_bps, elapsed_time);

    accrued_interest <= utilized_amount
}

// Simplified proptest block without the nested configuration attribute macro
proptest! {
    #[test]
    fn test_accrued_interest_never_exceeds_utilized_amount(
        utilized_amount in 0..100_000_000_000_i128,
        interest_rate_bps in 0..10_000_u32,
        elapsed_time in 0..315_360_000_u64,
    ) {
        prop_assert!(
            check_accrual_invariant(utilized_amount, interest_rate_bps, elapsed_time),
            "Safety invariant violated! Accrued interest exceeded the utilized principal amount."
        );
    }
}