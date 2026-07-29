# Requirements Document

## Introduction

This feature adds a `collateral.rs` module to the `creditra-credit` Soroban smart contract crate (`contracts/credit/src/collateral.rs`). The module introduces two capabilities that do not yet exist in the codebase:

1. **Dynamic LTV (Loan-to-Value) calculation** — computes the current LTV ratio for any borrower given their posted collateral and outstanding debt, and derives the maximum borrowable amount given an admin-configured LTV ceiling per collateral tier.
2. **Collateral withdrawal validation** — enforces that a borrower may only withdraw collateral when the remaining collateral keeps the post-withdrawal LTV at or below the configured ceiling.

The module must integrate seamlessly with the existing `CreditLineData`, `ContractError`, storage patterns (`DataKey` / `soroban_sdk` persistent/instance storage), `math_utils` arithmetic helpers, and event publishing conventions already established in the crate. All code must comply with `#![no_std]`, `wasm32-unknown-unknown`, and the strict security rules of the project.

---

## Glossary

- **Collateral_Module**: The new `contracts/credit/src/collateral.rs` module and its public entry points.
- **Collateral_Record**: The on-chain record keyed by borrower `Address` that stores `collateral_amount` (i128, native token units) and `collateral_asset` (Address of the posted asset token contract).
- **LTV_Config**: Admin-configurable instance-storage record holding `ltv_ceiling_bps` (u32, basis points, max 10 000) per collateral-asset address.
- **LTV_Ratio**: The ratio `utilized_amount / collateral_amount` expressed in basis points (0–10 000). Computed as `(utilized_amount × 10_000) / collateral_amount` using checked arithmetic.
- **Borrower**: The `Address` that owns a `CreditLineData` record in persistent storage.
- **Admin**: The address stored under the `"admin"` key in instance storage, set during `init`.
- **CollateralDepositedEvent**: Soroban event emitted on successful collateral deposit.
- **CollateralWithdrawnEvent**: Soroban event emitted on successful collateral withdrawal.
- **LtvConfigSetEvent**: Soroban event emitted when admin sets or updates an `LTV_Config`.
- **ContractError**: The existing `#[soroban_sdk::contracterror]` enum in `types.rs`; new variants will be added as required by this feature.
- **checked_add / checked_mul / checked_sub**: Rust checked arithmetic methods that return `None` on overflow instead of panicking or wrapping.
- **saturating_sub**: Rust saturating arithmetic that clamps to the type's minimum value instead of wrapping.
- **BPS_DENOMINATOR**: 10 000 — the number of basis points in 100%.

---

## Requirements

### Requirement 1: Collateral Storage and Types

**User Story:** As a protocol developer, I want collateral positions stored on-chain per borrower with a defined schema, so that all collateral operations have a single authoritative source of truth.

#### Acceptance Criteria

1. THE Collateral_Module SHALL define a `CollateralRecord` struct annotated with `#[contracttype]` containing the fields: `collateral_amount: i128` and `collateral_asset: soroban_sdk::Address`.
2. THE Collateral_Module SHALL store and retrieve `CollateralRecord` values from Soroban persistent storage keyed by a `DataKey::Collateral(Address)` variant that is added to the existing `DataKey` enum in `storage.rs`.
3. THE Collateral_Module SHALL define an `LtvConfig` struct annotated with `#[contracttype]` containing the field `ltv_ceiling_bps: u32`.
4. THE Collateral_Module SHALL store and retrieve `LtvConfig` values from Soroban instance storage keyed by a `DataKey::LtvConfig(Address)` variant (keyed on the collateral asset address).
5. THE Collateral_Module SHALL add the following new variants to `ContractError` in `types.rs`: `CollateralNotFound = 20`, `InsufficientCollateral = 21`, `LtvExceeded = 22`, `CollateralConfigNotFound = 23`, `CollateralAmountZero = 24`.

---

### Requirement 2: Admin LTV Configuration

**User Story:** As a protocol admin, I want to configure the maximum LTV ceiling per collateral asset, so that the contract can enforce safe collateral ratios automatically.

#### Acceptance Criteria

1. WHEN the admin calls `set_ltv_config(env, collateral_asset, ltv_ceiling_bps)`, THE Collateral_Module SHALL call `.require_auth()` on the admin address before executing any state change.
2. WHEN `set_ltv_config` is called with `ltv_ceiling_bps` greater than 10 000, THE Collateral_Module SHALL return `ContractError::RateTooHigh` without modifying storage.
3. WHEN `set_ltv_config` is called with `ltv_ceiling_bps` equal to 0, THE Collateral_Module SHALL return `ContractError::InvalidAmount` without modifying storage.
4. WHEN `set_ltv_config` is called with valid inputs, THE Collateral_Module SHALL store an `LtvConfig { ltv_ceiling_bps }` in instance storage under `DataKey::LtvConfig(collateral_asset)` and emit a `LtvConfigSetEvent`.
5. THE Collateral_Module SHALL expose a read-only `get_ltv_config(env, collateral_asset) -> Option<LtvConfig>` function that reads from instance storage without requiring authorization.

---

### Requirement 3: Collateral Deposit

**User Story:** As a borrower, I want to deposit collateral tokens into the contract, so that I can establish or increase my collateral position to support borrowing.

#### Acceptance Criteria

1. WHEN a borrower calls `deposit_collateral(env, borrower, collateral_asset, amount)`, THE Collateral_Module SHALL call `borrower.require_auth()` before executing any state change.
2. WHEN `deposit_collateral` is called with `amount` less than or equal to 0, THE Collateral_Module SHALL return `ContractError::InvalidAmount` without modifying storage.
3. WHEN `deposit_collateral` is called and the `amount` validation passes, IF no `LtvConfig` is configured for the `collateral_asset`, THE Collateral_Module SHALL return `ContractError::CollateralConfigNotFound` without modifying storage; WHEN both `amount` and config are invalid, THE Collateral_Module SHALL return the error corresponding to whichever validation is checked first (amount before config).
4. WHEN `deposit_collateral` is called with valid inputs, THE Collateral_Module SHALL transfer `amount` tokens from `borrower` to the contract address using the Soroban `token::Client` transfer interface.
5. WHEN a token transfer completes successfully, THE Collateral_Module SHALL add `amount` to the existing `CollateralRecord.collateral_amount` for the borrower (creating a new `CollateralRecord` with zero balance if none exists) using `checked_add`, returning `ContractError::Overflow` if the addition would overflow.
6. WHEN collateral is successfully deposited, THE Collateral_Module SHALL emit a `CollateralDepositedEvent` containing `borrower`, `collateral_asset`, `amount`, and `new_collateral_amount`; WHEN event publication fails after a successful deposit, THE Collateral_Module SHALL allow the deposit to complete without reverting.

---

### Requirement 4: Dynamic LTV Calculation

**User Story:** As a protocol developer or integrator, I want to compute the current LTV ratio and the maximum additional borrowing capacity for any borrower, so that draw validation and risk dashboards have accurate on-chain data.

#### Acceptance Criteria

1. THE Collateral_Module SHALL expose a pure (read-only, no auth) function `compute_ltv(env, borrower) -> Result<u32, ContractError>` that returns the current LTV in basis points.
2. WHEN `compute_ltv` is called and no `CollateralRecord` exists for the borrower OR `collateral_amount` is 0, THE Collateral_Module SHALL return `ContractError::CollateralNotFound`.
3. WHEN `compute_ltv` is called and `utilized_amount` is 0 (regardless of `collateral_amount`), THE Collateral_Module SHALL return `Ok(0u32)` and SHALL NOT execute the computation in AC 4.
4. WHEN both `utilized_amount` and `collateral_amount` are positive (i.e., neither is zero or negative), THE Collateral_Module SHALL compute LTV as `(utilized_amount × 10_000) / collateral_amount` using `checked_mul` on the `utilized_amount` operand, returning `ContractError::Overflow` if the multiplication overflows; WHEN either operand is not positive, this rule SHALL NOT apply and other acceptance criteria govern the response.
5. WHEN the computed LTV fraction exceeds 10 000 (i.e., debt exceeds collateral), THE Collateral_Module SHALL clamp the return value to 10 000 (representing 100% LTV).
6. THE Collateral_Module SHALL expose a read-only function `max_borrowable(env, borrower) -> Result<i128, ContractError>` that computes the maximum additional draw amount the borrower can take while staying at or below the configured LTV ceiling.
7. WHEN `max_borrowable` is called and no `LtvConfig` exists for the borrower's `collateral_asset`, THE Collateral_Module SHALL return `ContractError::CollateralConfigNotFound`.
8. WHEN `max_borrowable` is called and a valid `LtvConfig` exists, THE Collateral_Module SHALL compute `max_debt = (collateral_amount × ltv_ceiling_bps) / 10_000` using `checked_mul`, returning `ContractError::Overflow` on overflow; WHEN no valid config exists, AC 7 governs the response and this computation SHALL NOT be executed.
9. WHEN a valid `LtvConfig` exists and `max_debt` is greater than `utilized_amount`, THE Collateral_Module SHALL return `Ok(max_debt - utilized_amount)` as the remaining borrowing headroom.
10. WHEN a valid `LtvConfig` exists and `max_debt` is less than or equal to `utilized_amount` (borrower is already at or over the ceiling), THE Collateral_Module SHALL return `Ok(0)`.

---

### Requirement 5: Collateral Withdrawal Validation

**User Story:** As a borrower, I want to withdraw collateral I am no longer using, so that I can reclaim posted assets when my debt is sufficiently low.

#### Acceptance Criteria

1. WHEN a borrower calls `withdraw_collateral(env, borrower, amount)`, THE Collateral_Module SHALL call `borrower.require_auth()` before executing any state change.
2. WHEN `withdraw_collateral` is called with `amount` less than or equal to 0, THE Collateral_Module SHALL return `ContractError::InvalidAmount`.
3. WHEN `withdraw_collateral` is called and no `CollateralRecord` exists for the borrower, THE Collateral_Module SHALL return `ContractError::CollateralNotFound`.
4. WHEN `withdraw_collateral` is called with `amount` greater than `CollateralRecord.collateral_amount`, THE Collateral_Module SHALL return `ContractError::InsufficientCollateral`.
5. WHEN `withdraw_collateral` is called and no `LtvConfig` is configured for the borrower's `collateral_asset`, THE Collateral_Module SHALL return `ContractError::CollateralConfigNotFound`.
6. WHEN `withdraw_collateral` is called, THE Collateral_Module SHALL compute the post-withdrawal collateral as `collateral_amount - amount` using `checked_sub`, returning `ContractError::Overflow` on underflow.
7. WHEN the post-withdrawal collateral is 0 and `utilized_amount` is greater than 0, THE Collateral_Module SHALL return `ContractError::LtvExceeded` without modifying storage.
8. WHEN the post-withdrawal collateral is greater than 0 during a `withdraw_collateral` call, THE Collateral_Module SHALL compute the post-withdrawal LTV as `(utilized_amount × 10_000) / post_withdrawal_collateral` using `checked_mul`; WHEN the `checked_mul` result is `None` or the computed value is negative, THE Collateral_Module SHALL return `ContractError::Overflow`.
9. WHEN the post-withdrawal LTV exceeds `ltv_ceiling_bps`, THE Collateral_Module SHALL return `ContractError::LtvExceeded` without modifying storage or transferring tokens.
10. WHEN all validations pass, THE Collateral_Module SHALL reduce `CollateralRecord.collateral_amount` by `amount` using `checked_sub` and persist the updated record.
11. WHEN the record update persists, THE Collateral_Module SHALL transfer `amount` tokens from the contract address to `borrower` using the Soroban `token::Client`.
12. WHEN a withdrawal completes successfully, THE Collateral_Module SHALL emit a `CollateralWithdrawnEvent` containing `borrower`, `collateral_asset`, `amount`, and `new_collateral_amount`; WHEN event publication fails after a successful withdrawal, THE Collateral_Module SHALL allow the withdrawal to complete without reverting.

---

### Requirement 6: Overflow Safety and `no_std` Compliance

**User Story:** As a protocol developer, I want all arithmetic in the collateral module to be overflow-safe and `no_std`-compliant, so that the contract cannot be manipulated through integer overflow and can be compiled to WASM.

#### Acceptance Criteria

1. THE Collateral_Module SHALL use `checked_mul`, `checked_add`, or `checked_sub` for every arithmetic operation involving `collateral_amount`, `utilized_amount`, or `ltv_ceiling_bps`, returning `ContractError::Overflow` on any `None` result.
2. THE Collateral_Module SHALL NOT use `.unwrap()` or `.expect()` on any `Option` or `Result` produced within production code paths.
3. THE Collateral_Module SHALL NOT import any crate outside of `soroban_sdk` and the contract's own modules; all types use `soroban_sdk` primitives (`Address`, `i128`, `u32`, `Symbol`).
4. THE Collateral_Module SHALL maintain `#![no_std]` compliance; no `std::` types, allocators, or formatting macros that require `std` may be used.
5. WHEN overflow or underflow is detected during LTV computation, THE Collateral_Module SHALL return `ContractError::Overflow` rather than panicking.

---

### Requirement 7: Authorization Enforcement

**User Story:** As a protocol security reviewer, I want every state-changing entry point to require explicit authorization, so that no collateral operation can be performed without the correct signing key.

#### Acceptance Criteria

1. THE Collateral_Module SHALL call `.require_auth()` on `borrower` as the first statement inside `deposit_collateral` before any storage reads or writes.
2. THE Collateral_Module SHALL call `.require_auth()` on `borrower` as the first statement inside `withdraw_collateral` before any storage reads or writes.
3. THE Collateral_Module SHALL call `.require_auth()` on the admin address (retrieved via `auth::require_admin_auth`) as the first statement inside `set_ltv_config` before any storage reads or writes.
4. THE Collateral_Module SHALL NOT call `.require_auth()` inside read-only functions `compute_ltv`, `max_borrowable`, or `get_ltv_config`, as these are view functions.

---

### Requirement 8: Event Emission

**User Story:** As an indexer or analytics consumer, I want the collateral module to emit well-typed Soroban events on every state-changing operation, so that off-chain systems can track collateral positions in real time.

#### Acceptance Criteria

1. THE Collateral_Module SHALL define `CollateralDepositedEvent` as a `#[contracttype]` struct with fields: `borrower: Address`, `collateral_asset: Address`, `amount: i128`, `new_collateral_amount: i128`.
2. THE Collateral_Module SHALL define `CollateralWithdrawnEvent` as a `#[contracttype]` struct with fields: `borrower: Address`, `collateral_asset: Address`, `amount: i128`, `new_collateral_amount: i128`.
3. THE Collateral_Module SHALL define `LtvConfigSetEvent` as a `#[contracttype]` struct with fields: `collateral_asset: Address`, `ltv_ceiling_bps: u32`.
4. WHEN `deposit_collateral` succeeds, THE Collateral_Module SHALL publish a `CollateralDepositedEvent` under topic `(symbol_short!("credit"), symbol_short!("col_dep"))`.
5. WHEN `withdraw_collateral` succeeds, THE Collateral_Module SHALL publish a `CollateralWithdrawnEvent` under topic `(symbol_short!("credit"), symbol_short!("col_with"))`.
6. WHEN `set_ltv_config` succeeds, THE Collateral_Module SHALL publish a `LtvConfigSetEvent` under topic `(symbol_short!("credit"), symbol_short!("ltv_set"))`.

---

### Requirement 9: Rustdoc Documentation

**User Story:** As a developer maintaining or auditing the contract, I want all new and modified public items to have NatSpec-style rustdoc comments, so that the intent and behavior are clear without reading implementation code.

#### Acceptance Criteria

1. THE Collateral_Module SHALL include `///` rustdoc comments on every public function (`deposit_collateral`, `withdraw_collateral`, `set_ltv_config`, `compute_ltv`, `max_borrowable`, `get_ltv_config`) describing purpose, parameters, return values, errors, and authorization requirements.
2. THE Collateral_Module SHALL include `///` rustdoc comments on each new struct (`CollateralRecord`, `LtvConfig`, `CollateralDepositedEvent`, `CollateralWithdrawnEvent`, `LtvConfigSetEvent`) and each new `ContractError` variant.
3. THE Collateral_Module SHALL include a module-level `//!` doc comment describing the module's overall responsibility, security model, and integration points with the rest of the contract.

---

### Requirement 10: Unit Test Coverage

**User Story:** As a CI/CD pipeline, I want comprehensive unit tests for the collateral module that exercise happy paths, authorization failures, and arithmetic boundary conditions, so that regressions are caught automatically.

#### Acceptance Criteria

1. THE Collateral_Module SHALL include unit tests that verify `deposit_collateral` succeeds for a valid borrower, authorized caller, and configured asset, and that storage is updated correctly.
2. THE Collateral_Module SHALL include unit tests that verify `deposit_collateral` fails with `ContractError::Unauthorized` when called without `borrower.require_auth()` being satisfied.
3. THE Collateral_Module SHALL include unit tests that verify `withdraw_collateral` succeeds when the post-withdrawal LTV is strictly below the ceiling.
4. THE Collateral_Module SHALL include unit tests that verify `withdraw_collateral` returns `ContractError::LtvExceeded` when the withdrawal would push LTV above the configured ceiling.
5. THE Collateral_Module SHALL include unit tests that verify `withdraw_collateral` returns `ContractError::InsufficientCollateral` when the requested amount exceeds the posted collateral.
6. THE Collateral_Module SHALL include unit tests that verify `compute_ltv` returns 0 when `utilized_amount` is 0.
7. THE Collateral_Module SHALL include unit tests that verify `compute_ltv` returns the correct basis-point ratio for non-zero debt and collateral.
8. THE Collateral_Module SHALL include unit tests that verify `compute_ltv` clamps at 10 000 when debt exceeds collateral.
9. THE Collateral_Module SHALL include unit tests that verify `max_borrowable` returns 0 when the borrower is already at or above the LTV ceiling.
10. THE Collateral_Module SHALL include unit tests that verify `set_ltv_config` panics (Soroban auth failure) when called without admin authorization.
11. THE Collateral_Module SHALL include a boundary test with `collateral_amount = i128::MAX` and `utilized_amount = 1` confirming that `checked_mul` overflow in `compute_ltv` returns `ContractError::Overflow` rather than panicking.
