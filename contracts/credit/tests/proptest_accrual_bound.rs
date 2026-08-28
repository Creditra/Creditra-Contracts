use creditra_credit::math_utils::{prorate_interest, Rounding, SECONDS_PER_YEAR};
use proptest::prelude::*;

/// Asserts that for any valid borrower configuration over up to 1 year,
/// the accrued interest never exceeds the total utilized principal amount.
fn check_accrual_invariant(
    utilized_amount: i128,
    interest_rate_bps: u32,
    elapsed_time: u64,
) -> bool {
    let accrued_interest = prorate_interest(
        utilized_amount as u128,
        interest_rate_bps,
        elapsed_time,
        Rounding::Floor,
    ) as i128;

    accrued_interest <= utilized_amount
}

proptest! {
    #[test]
    fn test_accrued_interest_never_exceeds_utilized_amount(
        utilized_amount in 0..100_000_000_000_i128,
        interest_rate_bps in 0..10_000_u32,
        elapsed_time in 0..=SECONDS_PER_YEAR as u64,
    ) {
        prop_assert!(
            check_accrual_invariant(utilized_amount, interest_rate_bps, elapsed_time),
            "Safety invariant violated! Accrued interest exceeded the utilized principal amount."
        );
    }
}
