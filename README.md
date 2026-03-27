# Creditra Contracts

Core smart contracts for the Creditra protocol, managing credit lines, draw operations, repayments, and risk parameters.

## Features

* **Credit Line Management**: Open, suspend, reinstate, or close credit lines.
* **Draw Operations**: Borrowers can draw liquidity up to their credit limit from configured sources.
* **Repayments**: State-based repayment tracking (Token transfers handled via external integration).
* **Risk Engine Integration**: Admin-controlled updates for interest rates (BPS), credit limits, and risk scores.
* **Security**: Built-in reentrancy guards and strict administrative authorization checks.

## Onboarding & Reference

For a deep dive into the credit logic, status transitions, and mathematical models, see [docs/credit.md](docs/credit.md).

## Development

### Build
```bash
cargo build
```

### Test
```bash
cargo test -p creditra-credit
```

### Coverage
```bash
cargo llvm-cov --workspace --all-targets --fail-under-lines 95
```

## TODOs
* Implement automated interest accrual logic.
* Full integration of token pull-transfers for repay_credit.
