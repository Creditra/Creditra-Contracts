// SPDX-License-Identifier: MIT

//! Late-fee penalty model.
//!
//! # Overview
//!
//! Defines [`LateFeeConfig`], a two-variant enum representing the two
//! supported late-fee modes:
//!
//! - **[`LateFeeConfig::Flat`]** — a fixed token amount added once per missed
//!   installment, regardless of principal size or time elapsed.  The amount is
//!   stored in a [`FlatFeeConfig`] wrapper struct so the Soroban XDR codec can
//!   represent it as a tagged union without named-field variants (which the
//!   `#[contracttype]` macro does not support for enums in SDK v22).
//! - **[`LateFeeConfig::AprBased`]** — the existing APR-surcharge behaviour: a
//!   basis-point additive to the periodic interest rate, applied via
//!   [`crate::accrual`] when the line is delinquent.  The surcharge is stored
//!   in an [`AprFeeConfig`] wrapper struct.
//!
//! # Calculation
//!
//! [`compute_late_fee`] is a pure, deterministic function with no
//! floating-point arithmetic, no `unwrap`, and no side effects.  It is
//! called by the contract after each overdue installment is detected.
//!
//! # Backward compatibility
//!
//! `AprBased(AprFeeConfig { surcharge_bps: 0 })` is a no-op (zero fee),
//! matching the pre-604 default.  Any existing `PenaltySurchargeBps` storage
//! value is unaffected; callers that only use the legacy `PenaltySurchargeBps`
//! key continue to work without change.
//!
//! # API change summary (issue #604)
//!
//! | Before #604 | After #604 |
//! |---|---|
//! | Only `PenaltySurchargeBps` (APR-based) | Both APR-based *and* flat surcharge modes |
//! | No `LateFeeConfig` storage key | `DataKey::LateFeeConfig` stores the active mode |
//! | No `set_late_fee_config` / `get_late_fee_config` entrypoints | Both entrypoints added to `lib.rs` |
//!
//! ## Configuration examples
//!
//! ```ignore
//! // Flat mode: charge 50 tokens per missed installment
//! client.set_late_fee_config(&Some(LateFeeConfig::Flat(FlatFeeConfig { amount: 50 })));
//!
//! // APR-based mode: add 200 bps to the interest rate when delinquent
//! client.set_late_fee_config(&Some(LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 200 })));
//!
//! // Disable structured config (fall back to legacy PenaltySurchargeBps / LateFeeFlat)
//! client.set_late_fee_config(&None);
//! ```

use soroban_sdk::contracttype;

use crate::types::ContractError;

/// Payload for the [`LateFeeConfig::Flat`] variant.
///
/// Wraps the flat surcharge amount so the `#[contracttype]` macro on
/// [`LateFeeConfig`] can serialize the enum as an XDR tagged union.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FlatFeeConfig {
    /// Token units charged once per overdue installment.
    ///
    /// Must be `>= 0`. Zero disables the fee (no-op).
    pub amount: i128,
}

/// Payload for the [`LateFeeConfig::AprBased`] variant.
///
/// Wraps the APR surcharge in basis points so the `#[contracttype]` macro on
/// [`LateFeeConfig`] can serialize the enum as an XDR tagged union.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AprFeeConfig {
    /// Extra basis points added to the base interest rate while delinquent.
    ///
    /// Must be in `0..=10_000`.
    pub surcharge_bps: u32,
}

/// Configuration for the late-fee penalty applied to overdue installments.
///
/// # Variants
///
/// | Variant    | Behaviour |
/// |------------|-----------|
/// | `Flat`     | A fixed `amount` in token units is applied once per missed installment. |
/// | `AprBased` | An additive basis-point surcharge on the periodic interest rate while the line is delinquent. |
///
/// # Storage
///
/// Stored in instance storage under [`crate::storage::DataKey::LateFeeConfig`].
/// Admin-configurable via `set_late_fee_config` / `get_late_fee_config` on the
/// contract.  When the key is absent the contract falls back to the legacy
/// `LateFeeFlat` and `PenaltySurchargeBps` instance keys.
///
/// # Examples
///
/// ```ignore
/// // Flat: charge 50 tokens per missed installment
/// let cfg = LateFeeConfig::Flat(FlatFeeConfig { amount: 50 });
///
/// // APR-based: add 200 bps to the interest rate when delinquent
/// let cfg = LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 200 });
/// ```
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LateFeeConfig {
    /// Fixed flat amount charged once per overdue installment.
    ///
    /// Use [`FlatFeeConfig`] to supply the `amount`.  Zero disables the fee.
    Flat(FlatFeeConfig),
    /// Additive APR surcharge applied to delinquent lines during accrual.
    ///
    /// This preserves the existing `PenaltySurchargeBps` behaviour.
    /// `surcharge_bps` must be in `0..=10_000`.
    AprBased(AprFeeConfig),
}

/// Compute the flat late fee for `missed_installments` overdue periods.
///
/// Returns the total fee amount in token units.  All arithmetic is
/// overflow-safe: the computation uses `checked_mul` and propagates
/// [`ContractError::Overflow`] on overflow.
///
/// # Arguments
///
/// * `config` — Fee configuration.
/// * `missed_installments` — Number of overdue installment periods.  If
///   zero the function returns `Ok(0)` immediately.
///
/// # Returns
///
/// * `Ok(fee)` — total fee in token units (`>= 0`).
/// * `Err(ContractError::Overflow)` — arithmetic overflow detected.
/// * `Err(ContractError::InvalidAmount)` — `config` is `Flat` with a
///   negative `amount`.
///
/// # APR-based mode
///
/// The APR surcharge is applied by [`crate::accrual`], not here.  For
/// `AprBased` configs this function always returns `Ok(0)` — callers
/// should read `surcharge_bps` separately when they need it for accrual.
///
/// # Examples
///
/// ```ignore
/// let fee = compute_late_fee(LateFeeConfig::Flat(FlatFeeConfig { amount: 50 }), 3)?;
/// assert_eq!(fee, 150);
///
/// let fee = compute_late_fee(LateFeeConfig::AprBased(AprFeeConfig { surcharge_bps: 200 }), 3)?;
/// assert_eq!(fee, 0); // accrual handles APR surcharge separately
/// ```
pub fn compute_late_fee(
    config: LateFeeConfig,
    missed_installments: u64,
) -> Result<i128, ContractError> {
    if missed_installments == 0 {
        return Ok(0);
    }

    match config {
        LateFeeConfig::Flat(FlatFeeConfig { amount }) => {
            if amount < 0 {
                return Err(ContractError::InvalidAmount);
            }
            if amount == 0 {
                return Ok(0);
            }
            let count =
                i128::try_from(missed_installments).map_err(|_| ContractError::Overflow)?;
            amount.checked_mul(count).ok_or(ContractError::Overflow)
        }
        LateFeeConfig::AprBased(_) => {
            // APR surcharge is handled by crate::accrual; no flat amount here.
            Ok(0)
        }
    }
}
