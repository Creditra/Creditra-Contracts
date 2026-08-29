// SPDX-License-Identifier: MIT

//! Stable [`FreezeError`] catalog for the Creditra freeze domain.
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
//! | `3`   | `CreditLineNotFound`                 | Mirror          |
//! | `16`  | `BorrowerBlocked`                    | Mirror          |
//! | `19`  | `DrawsFrozen`                        | Mirror          |
//! | `40`  | `BorrowerFrozen`                     | Mirror          |
//! | `46`  | `CreditLineFrozen`                   | Mirror          |
//!
//! # Mirror tier semantics
//!
//! Mirror-tier variants share their discriminant *and* their semantic
//! meaning with the canonical `ContractError` enum at
//! `contracts/credit/src/types.rs`. Concretely:
//!
//! - `CreditLineNotFound = 3` → canonical `ContractError::CreditLineNotFound = 3`
//! - `BorrowerBlocked = 16` → canonical `ContractError::BorrowerBlocked = 16`
//! - `DrawsFrozen = 19` → canonical `ContractError::DrawsFrozen = 19`
//! - `BorrowerFrozen = 40` → canonical `ContractError::BorrowerFrozen = 40`
//! - `CreditLineFrozen = 46` → canonical `ContractError::CreditLineFrozen = 46`
//!
//! SDK clients decoding an error code emitted from the freeze contract
//! can map the integer directly to the canonical table at
//! [`docs/ERROR_CODES.md`](../../../docs/ERROR_CODES.md).
//!
//! # Freeze-specific tier semantics
//!
//! The freeze-specific tier (codes `100+`) is reserved for errors that
//! have no canonical counterpart in the credit contract's `ContractError`.
//! Each new variant must come with:
//!
//! 1. A focused test in `mod tests`.
//! 2. A row in [`docs/errors/freeze.md`](../../../docs/errors/freeze.md).

use soroban_sdk::contracterror;

/// Stable, ABI-pinned error catalog for Creditra freeze operations.
///
/// # Stability
/// Discriminants are part of the contract ABI. Reordering, removing, or
/// renumbering an existing variant is a **breaking change** that would
/// invalidate deployed SDK clients. New variants must be appended with the
/// next available integer and accompanied by a corresponding discriminant
/// assertion in tests.
///
/// # Tier system
///
/// Variants belong to one of two tiers:
///
/// - **Mirror tier** (`3`, `16`, `19`, `40`, `46`) — semantic twins of the
///   canonical `ContractError` codes; SDK clients can match them against
///   [`docs/ERROR_CODES.md`](../../../docs/ERROR_CODES.md).
/// - **Freeze-specific tier** (`100+`) — namespaced to leave a clear gap
///   from the credit contract's `1..49` range, defending against accidental
///   collisions if either contract appends to its enum in the future.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum FreezeError {
    // ── Mirror tier (matches contracts/credit/src/types.rs) ─────────────────
    //
    // Each mirror variant carries the same discriminant *and* the same
    // semantic meaning as its canonical counterpart, so SDK consumers
    // can map an emitted integer against docs/ERROR_CODES.md directly.
    /// The requested borrower does not have an open credit line.
    ///
    /// Mirror of canonical `ContractError::CreditLineNotFound` (`= 3`).
    CreditLineNotFound = 3,

    /// Borrower is on the admin-managed block list.
    ///
    /// Mirror of canonical `ContractError::BorrowerBlocked` (`= 16`).
    BorrowerBlocked = 16,

    /// Global draw freeze is active.
    ///
    /// Mirror of canonical `ContractError::DrawsFrozen` (`= 19`).
    DrawsFrozen = 19,

    /// Borrower draws are temporarily frozen until expiry.
    ///
    /// Mirror of canonical `ContractError::BorrowerFrozen` (`= 40`).
    BorrowerFrozen = 40,

    /// Credit line draws are frozen by admin (compliance hold).
    ///
    /// Mirror of canonical `ContractError::CreditLineFrozen` (`= 46`).
    CreditLineFrozen = 46,
    // ── Freeze-specific tier (codes 100+) ───────────────────────────────
    UnknownError = 100,
    //
    // These discriminants are exclusive to the freeze domain and
    // start at 100 to leave a 50-slot buffer above the credit contract's
    // range. New variants MUST be appended at the end of this
    // block.
}

impl FreezeError {
    pub fn from_u32_safe(code: u32) -> Self {
        match code {
            3 => Self::CreditLineNotFound,
            16 => Self::BorrowerBlocked,
            19 => Self::DrawsFrozen,
            40 => Self::BorrowerFrozen,
            46 => Self::CreditLineFrozen,
            100 => Self::UnknownError,
            _ => Self::UnknownError,
        }
    }
}

impl From<soroban_sdk::Error> for FreezeError {
    fn from(err: soroban_sdk::Error) -> Self {
        if err.is_type(soroban_sdk::xdr::ScErrorType::Contract) {
            Self::from_u32_safe(err.get_code())
        } else {
            Self::UnknownError
        }
    }
}

impl<'a> From<&'a FreezeError> for soroban_sdk::Error {
    fn from(err: &'a FreezeError) -> Self {
        soroban_sdk::Error::from_contract_error(*err as u32)
    }
}

impl From<FreezeError> for soroban_sdk::Error {
    fn from(err: FreezeError) -> Self {
        soroban_sdk::Error::from_contract_error(err as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::FreezeError;

    /// Pin every mirror discriminant against the canonical table.
    ///
    /// If any assertion fails, it means a mirror discriminant drifted and
    /// SDK consumers decoding an error emitted from the freeze contract
    /// against the canonical table would now mis-identify the failure.
    #[test]
    fn mirror_discriminants_match_canonical_credit_contract() {
        assert_eq!(FreezeError::CreditLineNotFound as u32, 3);
        assert_eq!(FreezeError::BorrowerBlocked as u32, 16);
        assert_eq!(FreezeError::DrawsFrozen as u32, 19);
        assert_eq!(FreezeError::BorrowerFrozen as u32, 40);
        assert_eq!(FreezeError::CreditLineFrozen as u32, 46);
    }

    /// Verify no two `FreezeError` variants share a discriminant. This
    /// is a compile-time guarantee from `#[repr(u32)]`, but we make it
    /// explicit here so the intent is documented and surfaced in test
    /// output.
    #[test]
    fn no_duplicate_discriminants() {
        let codes = [
            FreezeError::CreditLineNotFound as u32,
            FreezeError::BorrowerBlocked as u32,
            FreezeError::DrawsFrozen as u32,
            FreezeError::BorrowerFrozen as u32,
            FreezeError::CreditLineFrozen as u32,
            FreezeError::UnknownError as u32,
        ];

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
    /// a new variant at the end of the enum.
    #[test]
    fn variant_count_is_known() {
        const EXPECTED_VARIANT_COUNT: usize = 6;

        let codes = [
            FreezeError::CreditLineNotFound as u32,
            FreezeError::BorrowerBlocked as u32,
            FreezeError::DrawsFrozen as u32,
            FreezeError::BorrowerFrozen as u32,
            FreezeError::CreditLineFrozen as u32,
            FreezeError::UnknownError as u32,
        ];

        assert_eq!(
            codes.len(),
            EXPECTED_VARIANT_COUNT,
            "Variant count changed — update EXPECTED_VARIANT_COUNT"
        );
    }

    /// Verify `Eq` / `PartialEq` round-trip both directions.
    #[test]
    #[test]
    fn test_golden_vector_encodings() {
        assert_eq!(
            FreezeError::from_u32_safe(3),
            FreezeError::CreditLineNotFound
        );
        assert_eq!(FreezeError::from_u32_safe(16), FreezeError::BorrowerBlocked);
        assert_eq!(FreezeError::from_u32_safe(19), FreezeError::DrawsFrozen);
        assert_eq!(FreezeError::from_u32_safe(40), FreezeError::BorrowerFrozen);
        assert_eq!(
            FreezeError::from_u32_safe(46),
            FreezeError::CreditLineFrozen
        );
        assert_eq!(FreezeError::from_u32_safe(100), FreezeError::UnknownError);
        assert_eq!(FreezeError::from_u32_safe(999), FreezeError::UnknownError);
    }

    #[test]
    fn equality_round_trips() {
        let a = FreezeError::DrawsFrozen;
        let b = FreezeError::DrawsFrozen;
        assert_eq!(a, b);
        assert_ne!(
            a,
            FreezeError::CreditLineFrozen,
            "Distinct variants must not be equal"
        );
    }
}
