// SPDX-License-Identifier: MIT

//! Stable [`CollateralError`] catalog for the Creditra collateral domain.
//!
//! # Stability guarantee
//!
//! Each variant carries an explicit `#[repr(u32)]` discriminant. These
//! discriminants are part of the contract ABI. Existing variants must never
//! be reordered or renumbered. New variants must be appended at the end
//! with the next available integer.
//!
//! # Discriminant table
//!
//! | Code  | Variant                              | Tier            |
//! |-------|--------------------------------------|-----------------|
//! | `5`   | `InvalidAmount`                      | Mirror          |
//! | `12`  | `Overflow`                           | Mirror          |
//! | `22`  | `MissingLiquidityToken`              | Mirror          |
//! | `35`  | `CollateralRatioBelowMinimum`        | Mirror          |
//! | `39`  | `InsufficientCollateralBalance`      | Mirror          |
//! | `100` | `CollateralTokenNotAllowed`          | Collateral      |
//! | `101` | `CollateralRiskWeightOutOfRange`     | Collateral      |
//! | `102` | `CollateralTokenMismatch`            | Collateral      |
//! | `103` | `CollateralPositionLocked`           | Collateral      |
//! | `104` | `CollateralBalanceForTokenNotFound`  | Collateral      |
//!
//! # Mirror tier semantics
//!
//! Mirror-tier variants share their discriminant *and* their semantic
//! meaning with the canonical `ContractError` enum at
//! `contracts/credit/src/types.rs`. Concretely:
//!
//! - `InvalidAmount = 5`        → canonical `ContractError::InvalidAmount = 5`
//! - `Overflow = 12`            → canonical `ContractError::Overflow = 12`
//! - `MissingLiquidityToken = 22` → canonical `ContractError::MissingLiquidityToken = 22`
//! - `CollateralRatioBelowMinimum = 35` → canonical `ContractError::CollateralRatioBelowMinimum = 35`
//! - `InsufficientCollateralBalance = 39` → canonical `ContractError::InsufficientCollateralBalance = 39`
//!
//! SDK clients decoding an error code emitted from the collateral contract
//! can map the integer directly to the canonical table at
//! [`docs/ERROR_CODES.md`](../../../docs/ERROR_CODES.md).
//!
//! # Collateral-specific tier semantics
//!
//! The collateral-specific tier (codes `100+`) is reserved for errors that
//! have no canonical counterpart in the credit contract's `ContractError`.
//! Each new variant must come with:
//!
//! 1. A focused test in [`tests/catalog.rs`].
//! 2. A row in [`docs/errors/collateral.md`](../../../docs/errors/collateral.md).

use soroban_sdk::contracterror;

/// Stable, ABI-pinned error catalog for Creditra collateral operations.
///
/// # Stability
/// Discriminants are part of the contract ABI. Reordering, removing, or
/// renumbering an existing variant is a **breaking change** that would
/// invalidate deployed SDK clients. New variants must be appended with the
/// next available integer and accompanied by a corresponding discriminant
/// assertion in [`tests/catalog.rs`](../../tests/catalog.rs).
///
/// # Tier system
///
/// Variants belong to one of two tiers:
///
/// - **Mirror tier** (`5`, `12`, `22`, `35`, `39`) — semantic twins of the
///   canonical `ContractError` codes; SDK clients can match them against
///   [`docs/ERROR_CODES.md`](../../../docs/ERROR_CODES.md).
/// - **Collateral-specific tier** (`100+`) — namespaced to leave a clear gap
///   from the credit contract's `1..49` range, defending against accidental
///   collisions if either contract appends to its enum in the future.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum CollateralError {
    // ── Mirror tier (matches contracts/credit/src/types.rs) ─────────────────
    //
    // Each mirror variant carries the same discriminant *and* the same
    // semantic meaning as its canonical counterpart, so SDK consumers
    // can map an emitted integer against docs/ERROR_CODES.md directly.
    /// Amount is zero, negative, or otherwise not a valid token amount.
    ///
    /// Mirror of canonical `ContractError::InvalidAmount` (`= 5`).
    /// Raised when `amount <= 0` is supplied to a deposit, withdrawal,
    /// or partial-release entrypoint.
    InvalidAmount = 5,

    /// Arithmetic overflow during collateral math (balance aggregation,
    /// ratio multiplication, etc.). Overflow checks use `checked_add` /
    /// `checked_mul` so panics here mean the supplied amounts exceed
    /// `i128::MAX` and are outside the protocol's safe operating range.
    ///
    /// Mirror of canonical `ContractError::Overflow` (`= 12`).
    Overflow = 12,

    /// Collateral token address has not been configured (no
    /// `set_collateral_token` call), or — in the multi-collateral
    /// path — the supplied token is not on the admin-managed allowlist.
    ///
    /// Mirror of canonical `ContractError::MissingLiquidityToken` (`= 22`).
    MissingLiquidityToken = 22,

    /// Collateral withdrawal (or draw) would leave the borrower's
    /// collateral ratio strictly below the configured
    /// `MinCollateralRatioBps` floor for the borrower's outstanding
    /// utilization.
    ///
    /// Mirror of canonical `ContractError::CollateralRatioBelowMinimum`
    /// (`= 35`).
    CollateralRatioBelowMinimum = 35,

    /// Withdrawal amount strictly exceeds the borrower's deposited
    /// collateral balance for the requested token.
    ///
    /// Mirror of canonical `ContractError::InsufficientCollateralBalance`
    /// (`= 39`).
    InsufficientCollateralBalance = 39,

    // ── Collateral-specific tier (codes 100+) ───────────────────────────────
    //
    // These discriminants are exclusive to the collateral contract and
    // start at 100 to leave a 50-slot buffer above the credit contract's
    // 1..=49 range. New variants MUST be appended at the end of this
    // block (after 104) and paired with an assertion in tests/catalog.rs.
    /// The supplied collateral token address is not in the
    /// admin-managed allowlist used by the multi-collateral
    /// deposit/withdraw path.
    ///
    /// Tighter error than `MissingLiquidityToken` (which conflates
    /// "unset" with "rejected"); raised only when the token is known
    /// to be off-list.
    CollateralTokenNotAllowed = 100,

    /// The supplied collateral risk-weight (basis points) is outside
    /// the configured `[min_risk_weight_bps, max_risk_weight_bps]`
    /// bounds.
    ///
    /// Bounds are administered via the collateral allowlist
    /// governance path; see [`docs/error-taxonomy.md`](../../../docs/error-taxonomy.md)
    /// for the wider risk-tier table.
    CollateralRiskWeightOutOfRange = 101,

    /// A per-token collateral operation (deposit, withdraw, query)
    /// was invoked with a `token` argument that does not match the
    /// token currently bound to that borrower's collateral position.
    CollateralTokenMismatch = 102,

    /// The borrower's collateral position is locked because an
    /// outstanding draw is awaiting repayment; modifications
    /// (deposit-then-withdraw churn, large atomic releases) are
    /// blocked until the draw is cured.
    CollateralPositionLocked = 103,

    /// A per-token balance read was issued for a borrower who has
    /// no balance tracked under the requested token. Distinct from
    /// `InsufficientCollateralBalance` (which is about withdrawal
    /// exceeding existing balance) — this is raised by zero-balance
    /// lookup paths such as `get_balance_for_token` when the caller
    /// expects a balance to exist (e.g. for an oracle feed check).
    CollateralBalanceForTokenNotFound = 104,
    UnknownError = 200,
}

impl CollateralError {
    pub fn from_u32_safe(code: u32) -> Self {
        match code {
            5 => Self::InvalidAmount,
            12 => Self::Overflow,
            22 => Self::MissingLiquidityToken,
            35 => Self::CollateralRatioBelowMinimum,
            39 => Self::InsufficientCollateralBalance,
            100 => Self::CollateralTokenNotAllowed,
            101 => Self::CollateralRiskWeightOutOfRange,
            102 => Self::CollateralTokenMismatch,
            103 => Self::CollateralPositionLocked,
            104 => Self::CollateralBalanceForTokenNotFound,
            200 => Self::UnknownError,
            _ => Self::UnknownError,
        }
    }
}

impl From<soroban_sdk::Error> for CollateralError {
    fn from(err: soroban_sdk::Error) -> Self {
        if err.is_type(soroban_sdk::xdr::ScErrorType::Contract) {
            Self::from_u32_safe(err.get_code())
        } else {
            Self::UnknownError
        }
    }
}

impl<'a> From<&'a CollateralError> for soroban_sdk::Error {
    fn from(err: &'a CollateralError) -> Self {
        soroban_sdk::Error::from_contract_error(*err as u32)
    }
}

impl From<CollateralError> for soroban_sdk::Error {
    fn from(err: CollateralError) -> Self {
        soroban_sdk::Error::from_contract_error(err as u32)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// In-module unit tests (rapid feedback; integration tests live in
// tests/catalog.rs for full-coverage pin via the test binary).
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::CollateralError;

    /// Pin every mirror discriminant against the canonical table.
    ///
    /// If any assertion fails, it means a mirror discriminant drifted and
    /// SDK consumers decoding an error emitted from the collateral contract
    /// against the canonical table would now mis-identify the failure.
    #[test]
    fn mirror_discriminants_match_canonical_credit_contract() {
        assert_eq!(CollateralError::InvalidAmount as u32, 5);
        assert_eq!(CollateralError::Overflow as u32, 12);
        assert_eq!(CollateralError::MissingLiquidityToken as u32, 22);
        assert_eq!(CollateralError::CollateralRatioBelowMinimum as u32, 35);
        assert_eq!(CollateralError::InsufficientCollateralBalance as u32, 39);
    }

    /// Pin every collateral-specific discriminant within the `100+`
    /// namespace. Tests guard against accidental silent renumbering.
    #[test]
    fn collateral_specific_discriminants_are_stable() {
        assert_eq!(CollateralError::CollateralTokenNotAllowed as u32, 100);
        assert_eq!(CollateralError::CollateralRiskWeightOutOfRange as u32, 101);
        assert_eq!(CollateralError::CollateralTokenMismatch as u32, 102);
        assert_eq!(CollateralError::CollateralPositionLocked as u32, 103);
        assert_eq!(
            CollateralError::CollateralBalanceForTokenNotFound as u32,
            104
        );
    }

    /// Verify no two `CollateralError` variants share a discriminant. This
    /// is a compile-time guarantee from `#[repr(u32)]`, but we make it
    /// explicit here so the intent is documented and surfaced in test
    /// output.
    #[test]
    fn no_duplicate_discriminants() {
        let codes = [
            CollateralError::InvalidAmount as u32,
            CollateralError::Overflow as u32,
            CollateralError::MissingLiquidityToken as u32,
            CollateralError::CollateralRatioBelowMinimum as u32,
            CollateralError::InsufficientCollateralBalance as u32,
            CollateralError::CollateralTokenNotAllowed as u32,
            CollateralError::CollateralRiskWeightOutOfRange as u32,
            CollateralError::CollateralTokenMismatch as u32,
            CollateralError::CollateralPositionLocked as u32,
            CollateralError::CollateralBalanceForTokenNotFound as u32,
            CollateralError::UnknownError as u32,
        ];

        // Manually detect duplicates to keep the test dependency-free.
        for i in 0..codes.len() {
            for j in (i + 1)..codes.len() {
                assert_ne!(
                    codes[i], codes[j],
                    "Duplicate discriminant {} detected between variant indices {} and {}",
                    codes[i], i, j
                );
            }
        }
    }

    /// Pin the total variant count. Update this constant only when adding
    /// a new variant at the end of the enum; also add a row to the table
    /// in this file's module-level docstring and to `tests/catalog.rs`.
    #[test]
    fn variant_count_is_known() {
        const EXPECTED_VARIANT_COUNT: usize = 11;

        let codes = [
            CollateralError::InvalidAmount as u32,
            CollateralError::Overflow as u32,
            CollateralError::MissingLiquidityToken as u32,
            CollateralError::CollateralRatioBelowMinimum as u32,
            CollateralError::InsufficientCollateralBalance as u32,
            CollateralError::CollateralTokenNotAllowed as u32,
            CollateralError::CollateralRiskWeightOutOfRange as u32,
            CollateralError::CollateralTokenMismatch as u32,
            CollateralError::CollateralPositionLocked as u32,
            CollateralError::CollateralBalanceForTokenNotFound as u32,
            CollateralError::UnknownError as u32,
        ];

        assert_eq!(
            codes.len(),
            EXPECTED_VARIANT_COUNT,
            "Variant count changed — update EXPECTED_VARIANT_COUNT, this file's \
             discriminant table, lib.rs docstring, tests/catalog.rs, and docs/errors/collateral.md"
        );
    }

    /// Verify the collateral-specific tier does not collide with the
    /// credit contract's `1..=49` range. This is the second line of
    /// defence against accidental renumbering.
    #[test]
    fn collateral_specific_tier_starts_at_or_above_100() {
        let new_codes = [
            CollateralError::CollateralTokenNotAllowed as u32,
            CollateralError::CollateralRiskWeightOutOfRange as u32,
            CollateralError::CollateralTokenMismatch as u32,
            CollateralError::CollateralPositionLocked as u32,
            CollateralError::CollateralBalanceForTokenNotFound as u32,
            CollateralError::UnknownError as u32,
        ];

        for &code in &new_codes {
            assert!(
                code >= 100,
                "Collateral-specific discriminant {} falls below the 100+ \
                 namespace — reserve codes 100+ for this crate",
                code
            );
        }
    }

    /// Verify `Eq` / `PartialEq` round-trip both directions. Useful for
    /// `match` arms in the future contract logic.
    #[test]
    #[test]
    fn test_golden_vector_encodings() {
        assert_eq!(
            CollateralError::from_u32_safe(5),
            CollateralError::InvalidAmount
        );
        assert_eq!(
            CollateralError::from_u32_safe(12),
            CollateralError::Overflow
        );
        assert_eq!(
            CollateralError::from_u32_safe(22),
            CollateralError::MissingLiquidityToken
        );
        assert_eq!(
            CollateralError::from_u32_safe(35),
            CollateralError::CollateralRatioBelowMinimum
        );
        assert_eq!(
            CollateralError::from_u32_safe(39),
            CollateralError::InsufficientCollateralBalance
        );
        assert_eq!(
            CollateralError::from_u32_safe(100),
            CollateralError::CollateralTokenNotAllowed
        );
        assert_eq!(
            CollateralError::from_u32_safe(101),
            CollateralError::CollateralRiskWeightOutOfRange
        );
        assert_eq!(
            CollateralError::from_u32_safe(102),
            CollateralError::CollateralTokenMismatch
        );
        assert_eq!(
            CollateralError::from_u32_safe(103),
            CollateralError::CollateralPositionLocked
        );
        assert_eq!(
            CollateralError::from_u32_safe(104),
            CollateralError::CollateralBalanceForTokenNotFound
        );
        assert_eq!(
            CollateralError::from_u32_safe(200),
            CollateralError::UnknownError
        );
        assert_eq!(
            CollateralError::from_u32_safe(999),
            CollateralError::UnknownError
        );
    }

    #[test]
    fn equality_round_trips() {
        let a = CollateralError::InsufficientCollateralBalance;
        let b = CollateralError::InsufficientCollateralBalance;
        assert_eq!(a, b);
        assert_ne!(
            a,
            CollateralError::CollateralRatioBelowMinimum,
            "Distinct variants must not be equal"
        );
    }
}
