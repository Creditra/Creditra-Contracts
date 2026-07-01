# Interest Accrual Design Specification

**Version: 2026-04-25 (Design Specification)**
**Issue: #119 - On-chain interest accrual design**

## Executive Summary

This document provides a comprehensive design specification for on-chain interest accrual in the Creditra credit contract. It covers the mathematical model, implementation approach, edge cases, and migration strategy from the current no-accrual baseline.

## Current State Analysis

### Existing Infrastructure
- `CreditLineData` already stores `accrued_interest` and `last_accrual_ts` fields
- Basic accrual logic exists in `accrual.rs` with `apply_accrual` function
- Interest calculation uses simple interest formula with basis points
- Events are emitted for interest accrual (`InterestAccruedEvent`)

### Current Implementation Formula
```rust
accrued = floor(utilized_amount * interest_rate_bps * elapsed_seconds / (10_000 * 31_536_000))
```

## Design Requirements

### Functional Requirements
1. **Deterministic Calculation**: Interest must be calculated deterministically from ledger timestamps
2. **Simple Interest Model**: Use non-compounding simple interest for predictable accrual
3. **Lazy Evaluation**: Accrual computed only when credit line is touched (no background jobs)
4. **Backward Compatibility**: Existing lines with `last_accrual_ts == 0` must not accrue retroactive interest
5. **Event Transparency**: Emit explicit events when interest is materialized

### Non-Functional Requirements
1. **Gas Efficiency**: Minimize computational overhead for accrual calculations
2. **Overflow Safety**: All arithmetic must use checked operations
3. **Compliance Alignment**: Support regulatory requirements for interest calculation
4. **Audit Trail**: Complete event history for interest accrual

## Mathematical Model

### Core Formula
**Simple Interest per Second:**
```
daily_rate = interest_rate_bps / 10_000 / 365
secondly_rate = daily_rate / 86_400
accrued_interest = floor(principal * secondly_rate * elapsed_seconds)
```

**Simplified Integer Implementation:**
```
accrued = floor(principal * interest_rate_bps * elapsed_seconds / (10_000 * 31_536_000))
```

### Constants
- `SECONDS_PER_YEAR = 31_536_000` (365 days, non-leap year)
- `BASIS_POINTS_DIVISOR = 10_000`
- `ROUNDING_MODE = floor` (always round down, favor borrower)

### Rounding Policy
- **Floor Rounding**: Always round down to favor borrowers
- **Minimum Accrual Threshold**: Only accrue when result ≥ 1 unit
- **Dust Handling**: Fractional amounts remain unmaterialized until threshold met

## Implementation Architecture

### Accrual Checkpoint System

#### Initialization Rules
```rust
if line.last_accrual_ts == 0 {
    // First time accrual - establish checkpoint without retroactive charging
    line.last_accrual_ts = current_timestamp;
    return line;
}
```

#### Accrual Trigger Points
Interest accrual is applied before state mutations in:
1. `draw_credit` - Before drawing new funds
2. `repay_credit` - Before applying repayment
3. `update_risk_parameters` - Before parameter changes
4. `suspend_credit_line` - Before status change
5. `default_credit_line` - Before defaulting
6. `close_credit_line` - Before closure
7. `reinstate_credit_line` - Before reinstatement

#### Status-Specific Behavior
| Status | Accrual Behavior | Rationale |
|--------|------------------|-----------|
| `Active` | Normal accrual | Standard operating state |
| `Suspended` | Normal accrual | Time passes regardless of status |
| `Defaulted` | Normal accrual (v1) | Contractual rate continues |
| `Closed` | No accrual | Line is terminated |
| `Restricted` | Normal accrual | Debt continues to accrue |

### Grace Period Integration

#### Suspended Line Grace Period
When grace period policy is configured:
```rust
if line.status == Suspended && grace_config.exists() {
    let grace_elapsed = min(elapsed, grace_period_seconds);
    let post_grace_elapsed = elapsed - grace_elapsed;
    
    // Apply waiver rate during grace period
    let grace_accrued = calculate_interest(principal, waiver_rate, grace_elapsed);
    // Apply full rate after grace period
    let post_grace_accrued = calculate_interest(principal, full_rate, post_grace_elapsed);
    
    total_accrued = grace_accrued + post_grace_accrued;
}
```

## Data Structures

### Enhanced CreditLineData
```rust
pub struct CreditLineData {
    // Existing fields...
    pub borrower: Address,
    pub credit_limit: i128,
    pub utilized_amount: i128,
    pub interest_rate_bps: u32,
    pub risk_score: u32,
    pub status: CreditStatus,
    pub last_rate_update_ts: u64,
    pub accrued_interest: i128,
    pub last_accrual_ts: u64,  // 0 = not initialized
    
    // New fields for enhanced accrual
    pub total_accrual_periods: u64,     // Count of accrual calculations
    pub first_accrual_ts: u64,         // Timestamp of first accrual
    pub last_accrual_amount: i128,     // Amount from last accrual event
}
```

### Grace Period Configuration
```rust
pub struct GracePeriodConfig {
    pub grace_period_seconds: u64,
    pub waiver_mode: GraceWaiverMode,
    pub reduced_rate_bps: u32,
}

pub enum GraceWaiverMode {
    FullWaiver,      // 0% interest during grace period
    ReducedRate,    // Reduced rate during grace period
}
```

## Event Model

### InterestAccruedEvent
```rust
pub struct InterestAccruedEvent {
    pub borrower: Address,
    pub accrued_amount: i128,           // Newly accrued this period
    pub total_accrued_interest: i128,   // Cumulative accrued interest
    pub new_utilized_amount: i128,      // Total debt after accrual
    pub elapsed_seconds: u64,           // Time period for calculation
    pub effective_rate_bps: u32,         // Rate actually applied
    pub timestamp: u64,
}
```

### Event Emission Rules
- Emit only when `accrued_amount > 0`
- Include complete calculation context for auditability
- Emit during grace periods with effective rate details

## Edge Cases and Handling

### Zero Utilization
```rust
if line.utilized_amount == 0 {
    // Update timestamp but no interest calculation
    line.last_accrual_ts = now;
    return line;
}
```

### Overflow Protection
```rust
let intermediate = utilized
    .checked_mul(rate)
    .and_then(|v| v.checked_mul(seconds));

if intermediate.is_none() {
    panic!("interest calculation overflow");
}
```

### Timestamp Edge Cases
- **Backwards Time**: If `now <= last_accrual_ts`, no accrual
- **Large Jumps**: Handle multi-year elapsed periods correctly
- **Leap Years**: Use fixed 365-day year for consistency

### Rate Changes During Period
- Rate changes take effect at next accrual checkpoint
- No retroactive application for periods already elapsed
- Clear event trail for rate change timing

## Migration Strategy

### Phase 1: Storage Migration
- Add new fields to `CreditLineData` with default values
- Set `last_accrual_ts = 0` for all existing lines
- Initialize `total_accrual_periods = 0`

### Phase 2: Activation
- Deploy contract with accrual logic enabled
- First touch on each line establishes accrual checkpoint
- No retroactive interest charged for pre-existing lines

### Phase 3: Monitoring
- Monitor accrual calculation accuracy
- Validate event emission completeness
- Confirm gas costs are acceptable

## Security Considerations

### Trust Boundaries
- **Ledger Timestamp**: Trust Soroban host for monotonic time
- **Rate Changes**: Admin-controlled with rate-change limits
- **Grace Period**: Admin-configurable policy

### Attack Vectors
1. **Timestamp Manipulation**: Mitigated by Soroban host controls
2. **Rate Change Abuse**: Limited by rate-change configuration
3. **Overflow Attacks**: Protected by checked arithmetic
4. **Reentrancy**: Guarded in draw/repay functions

### Failure Modes
| Failure Mode | Impact | Mitigation |
|--------------|--------|------------|
| Timestamp rollback | No accrual | Check monotonicity |
| Arithmetic overflow | Transaction revert | Checked operations |
| Storage corruption | Data loss | Schema validation |

## Testing Strategy

### Unit Tests
1. **Basic Accrual**: Verify formula correctness
2. **Zero Utilization**: Confirm no accrual when no debt
3. **Initialization**: Proper checkpoint establishment
4. **Rate Changes**: Accurate handling of rate updates
5. **Grace Period**: Correct waiver application
6. **Edge Cases**: Overflow, timestamp issues

### Integration Tests
1. **End-to-End Flows**: Draw → Accrual → Repay cycles
2. **Status Transitions**: Accrual across status changes
3. **Multi-Period**: Accrual over multiple checkpoints
4. **Gas Analysis**: Performance under various conditions

### Property Tests
1. **Monotonicity**: Interest never decreases
2. **Bounds**: Accrued amount within mathematical limits
3. **Consistency**: Same inputs produce same outputs

## Compliance Alignment

### Regulatory Requirements
1. **Truth in Lending**: Clear interest calculation methodology
2. **Audit Trail**: Complete event history for regulators
3. **Disclosure**: Transparent rate and fee structure
4. **Fair Practices**: Rounding favors borrowers

### Reporting Capabilities
- Total interest accrued per borrower
- Effective APR calculations
- Accrual period breakdowns
- Grace period impact analysis

## Performance Analysis

### Gas Cost Estimates
- Base accrual calculation: ~15,000 gas
- Grace period logic: +5,000 gas
- Event emission: +8,000 gas
- Total per accrual: ~28,000 gas

### Optimization Opportunities
1. **Batch Processing**: Process multiple lines in single transaction
2. **Rate Caching**: Pre-compute rate factors
3. **Event Batching**: Aggregate events for efficiency

## Future Enhancements

### Version 2 Considerations
1. **Compound Interest**: Optional compounding periods
2. **Variable Rates**: Time-based rate schedules
3. **Penalty Rates**: Higher rates for defaulted lines
4. **Interest-Only Payments**: Support for interest-only periods

### Extensibility Points
- Pluggable rounding modes
- Custom accrual periods (daily, weekly)
- Multi-currency rate support
- Integration with external rate oracles

## Open Questions

### Design Decisions Pending
1. **Defaulted Line Rates**: Should defaulted lines pay penalty rates?
2. **Accrual Frequency**: Should we support proactive accrual entrypoints?
3. **Minimum Threshold**: Should there be a minimum accrual amount?
4. **Rate Change Timing**: Immediate vs. next checkpoint application?

### Implementation Considerations
1. **Storage Migration**: How to handle existing lines efficiently?
2. **Event Volume**: Impact on indexer storage requirements
3. **Gas Limits**: Maximum lines per accrual transaction
4. **Testing Scope**: Comprehensive test coverage requirements

## Conclusion

This design specification provides a robust, compliant, and efficient framework for on-chain interest accrual in the Creditra protocol. The simple interest model with lazy evaluation ensures predictable gas costs while maintaining auditability and regulatory compliance.

The implementation prioritizes borrower protection through floor rounding and comprehensive event logging, while providing sufficient flexibility for future enhancements through well-designed extension points.

## Appendix

### A. Mathematical Derivations
Detailed derivation of the interest formula and edge case handling.

### B. Test Case Matrix
Comprehensive list of test scenarios and expected outcomes.

### C. Migration Scripts
Example scripts for storage migration and activation.

### D. Performance Benchmarks
Detailed gas cost analysis and optimization recommendations.
