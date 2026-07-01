// Standalone test for interest accrual functionality
// This test validates the core accrual logic independently

use std::time::{SystemTime, UNIX_EPOCH};

// Constants from the contract
const SECONDS_PER_YEAR: u64 = 31_536_000; // 365 days
const BASIS_POINTS_DIVISOR: u32 = 10_000;

// Simplified CreditLineData for testing
#[derive(Debug, Clone)]
struct TestCreditLine {
    utilized_amount: i128,
    interest_rate_bps: u32,
    accrued_interest: i128,
    last_accrual_ts: u64,
}

fn calculate_accrual(line: &TestCreditLine, now: u64) -> (i128, TestCreditLine) {
    // Handle initialization
    if line.last_accrual_ts == 0 {
        let mut updated = line.clone();
        updated.last_accrual_ts = now;
        return (0, updated);
    }
    
    // No time elapsed
    if now <= line.last_accrual_ts {
        return (0, line.clone());
    }
    
    // No debt
    if line.utilized_amount == 0 {
        let mut updated = line.clone();
        updated.last_accrual_ts = now;
        return (0, updated);
    }
    
    let elapsed = now - line.last_accrual_ts;
    
    // Formula: accrued = floor(utilized_amount * interest_rate_bps * elapsed_seconds / (10_000 * 31_536_000))
    let utilized = line.utilized_amount;
    let rate = line.interest_rate_bps as i128;
    let seconds = elapsed as i128;
    
    let denominator: i128 = BASIS_POINTS_DIVISOR as i128 * (SECONDS_PER_YEAR as i128);
    
    // Use checked multiplication to prevent overflow
    let intermediate = utilized.checked_mul(rate)
        .and_then(|v| v.checked_mul(seconds));
    
    if let Some(val) = intermediate {
        let accrued = val / denominator;
        
        if accrued > 0 {
            let mut updated = line.clone();
            updated.utilized_amount = updated.utilized_amount + accrued;
            updated.accrued_interest = updated.accrued_interest + accrued;
            updated.last_accrual_ts = now;
            return (accrued, updated);
        }
    }
    
    // No accrual or overflow
    let mut updated = line.clone();
    updated.last_accrual_ts = now;
    (0, updated)
}

fn main() {
    println!("=== Interest Accrual Test Suite ===\n");
    
    // Test 1: Basic accrual calculation
    println!("Test 1: Basic accrual calculation");
    let line = TestCreditLine {
        utilized_amount: 1000,
        interest_rate_bps: 300, // 3%
        accrued_interest: 0,
        last_accrual_ts: 1000,
    };
    
    let (accrued, updated) = calculate_accrual(&line, 1000 + SECONDS_PER_YEAR);
    println!("  Principal: 1000, Rate: 3%, Period: 1 year");
    println!("  Expected accrual: floor(1000 * 300 * 31_536_000 / (10_000 * 31_536_000)) = 30");
    println!("  Actual accrual: {}", accrued);
    assert_eq!(accrued, 30, "Basic accrual calculation failed");
    println!("  ✅ PASSED\n");
    
    // Test 2: Zero utilization
    println!("Test 2: Zero utilization");
    let line = TestCreditLine {
        utilized_amount: 0,
        interest_rate_bps: 300,
        accrued_interest: 0,
        last_accrual_ts: 1000,
    };
    
    let (accrued, _) = calculate_accrual(&line, 2000);
    println!("  Principal: 0, Rate: 3%, Period: 1000 seconds");
    println!("  Expected accrual: 0");
    println!("  Actual accrual: {}", accrued);
    assert_eq!(accrued, 0, "Zero utilization should not accrue");
    println!("  ✅ PASSED\n");
    
    // Test 3: First-time initialization
    println!("Test 3: First-time initialization");
    let line = TestCreditLine {
        utilized_amount: 1000,
        interest_rate_bps: 300,
        accrued_interest: 0,
        last_accrual_ts: 0, // Not initialized
    };
    
    let (accrued, updated) = calculate_accrual(&line, 1000);
    println!("  Initial last_accrual_ts: 0");
    println!("  Expected accrual: 0 (no retroactive charging)");
    println!("  Actual accrual: {}", accrued);
    assert_eq!(accrued, 0, "First-time should not accrue");
    assert_eq!(updated.last_accrual_ts, 1000, "Checkpoint should be set");
    println!("  ✅ PASSED\n");
    
    // Test 4: Small time period (floor rounding)
    println!("Test 4: Small time period (floor rounding)");
    let line = TestCreditLine {
        utilized_amount: 1000,
        interest_rate_bps: 300,
        accrued_interest: 0,
        last_accrual_ts: 1000,
    };
    
    let (accrued, _) = calculate_accrual(&line, 1001); // 1 second
    println!("  Principal: 1000, Rate: 3%, Period: 1 second");
    let expected = (1000i64 * 300i64 * 1i64) / (10_000i64 * 31_536_000i64);
    println!("  Expected accrual: floor({} / {}) = 0", 1000i64 * 300i64 * 1i64, 10_000i64 * 31_536_000i64);
    println!("  Actual accrual: {}", accrued);
    assert_eq!(accrued, 0, "Small period should round down to 0");
    println!("  ✅ PASSED\n");
    
    // Test 5: Multi-period accrual
    println!("Test 5: Multi-period accrual");
    let mut line = TestCreditLine {
        utilized_amount: 1000,
        interest_rate_bps: 300,
        accrued_interest: 0,
        last_accrual_ts: 1000,
    };
    
    // First period: 6 months
    let six_months = SECONDS_PER_YEAR / 2;
    let (accrued1, line1) = calculate_accrual(&line, 1000 + six_months);
    println!("  Period 1: 6 months, accrued: {}", accrued1);
    
    // Second period: 6 more months
    let (accrued2, line2) = calculate_accrual(&line1, 1000 + 2 * six_months);
    println!("  Period 2: 6 months, accrued: {}", accrued2);
    
    let total_accrued = accrued1 + accrued2;
    println!("  Total accrued: {}", total_accrued);
    println!("  Expected: ~30 (should be close to 1 year at 3%)");
    
    // Should be approximately 30 (some rounding differences)
    assert!(total_accrued >= 29 && total_accrued <= 31, "Multi-period accrual failed: {}", total_accrued);
    println!("  ✅ PASSED\n");
    
    // Test 6: Different rates
    println!("Test 6: Different interest rates");
    let rates = vec![100, 500, 1000, 2000]; // 1%, 5%, 10%, 20%
    
    for rate in rates {
        let line = TestCreditLine {
            utilized_amount: 1000,
            interest_rate_bps: rate,
            accrued_interest: 0,
            last_accrual_ts: 1000,
        };
        
        let (accrued, _) = calculate_accrual(&line, 1000 + SECONDS_PER_YEAR);
        let expected = (1000 * rate) / 10_000; // Simplified for 1 year
        println!("  Rate: {}% ({} bps), Expected: {}, Actual: {}", 
                rate / 100, rate, expected, accrued);
        assert_eq!(accrued, expected as i128, "Rate calculation failed for {} bps", rate);
    }
    println!("  ✅ PASSED\n");
    
    println!("=== All Tests Passed! ===");
    println!("Interest accrual implementation is working correctly.");
}
