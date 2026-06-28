// SPDX-License-Identifier: MIT

//! Protocol fee skim split between treasury and bounty pools.
//!
//! When a borrower repays, the protocol fee (`ProtocolFeeBps`) is
//! skimmed from the total repayment amount into the contract and allocated
//! between two accumulators by [`TreasuryFeeShareBps`]:
//!
//! - **Treasury** — withdrawable via `propose_treasury_withdrawal` / `execute_treasury_withdrawal` to `TreasuryAddress`.
//! - **Bounty pool** — withdrawable via `withdraw_bounty` to `BountyAddress`.
//!
//! The treasury share is computed with floor rounding; the bounty pool receives
//! the remainder so no tokens are lost to integer division.

use crate::math_utils::{apply_bps, Rounding};
use soroban_sdk::{Address, Env};

/// Maximum basis points for a fee-share ratio (100 %).
pub const MAX_FEE_SHARE_BPS: u32 = 10_000;

/// Default treasury share when unset: 100 % to treasury (backward compatible).
pub const DEFAULT_TREASURY_FEE_SHARE_BPS: u32 = 10_000;

/// Result of splitting a protocol fee between treasury and bounty accumulators.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FeeSplitAmounts {
    /// Portion credited to `TreasuryBalance`.
    pub treasury_amount: i128,
    /// Portion credited to `BountyBalance`.
    pub bounty_amount: i128,
}

/// Split `total_fee` by `treasury_share_bps` in the range `0..=10_000`.
///
/// Treasury receives `floor(total_fee * treasury_share_bps / 10_000)`; the
/// bounty pool receives the remainder.
pub fn split_protocol_fee(total_fee: i128, treasury_share_bps: u32) -> FeeSplitAmounts {
    if total_fee <= 0 {
        return FeeSplitAmounts {
            treasury_amount: 0,
            bounty_amount: 0,
        };
    }

    if treasury_share_bps == 0 {
        return FeeSplitAmounts {
            treasury_amount: 0,
            bounty_amount: total_fee,
        };
    }

    if treasury_share_bps >= MAX_FEE_SHARE_BPS {
        return FeeSplitAmounts {
            treasury_amount: total_fee,
            bounty_amount: 0,
        };
    }

    let treasury_amount =
        apply_bps(total_fee as u128, treasury_share_bps, Rounding::Floor) as i128;
    let bounty_amount = total_fee - treasury_amount;

    FeeSplitAmounts {
        treasury_amount,
        bounty_amount,
    }
}

/// Return configured treasury fee share in basis points.
///
/// Defaults to [`DEFAULT_TREASURY_FEE_SHARE_BPS`] when unset.
pub fn get_treasury_fee_share_bps(env: &Env) -> u32 {
    crate::storage::get_treasury_fee_share_bps(env).unwrap_or(DEFAULT_TREASURY_FEE_SHARE_BPS)
}

/// Credit a skimmed protocol fee to treasury and bounty accumulators and emit
/// [`crate::events::FeeAccruedEvent`].
pub fn accrue_protocol_fee(env: &Env, borrower: &Address, total_fee: i128) {
    if total_fee <= 0 {
        return;
    }

    let split = split_protocol_fee(total_fee, get_treasury_fee_share_bps(env));

    if split.treasury_amount > 0 {
        crate::storage::add_treasury_balance(env, split.treasury_amount);
    }
    if split.bounty_amount > 0 {
        crate::storage::add_bounty_balance(env, split.bounty_amount);
    }

    crate::events::publish_fee_accrued_event(
        env,
        crate::events::FeeAccruedEvent {
            borrower: borrower.clone(),
            fee_amount: total_fee,
            treasury_amount: split.treasury_amount,
            bounty_amount: split.bounty_amount,
            new_treasury_balance: crate::storage::get_treasury_balance(env),
            new_bounty_balance: crate::storage::get_bounty_balance(env),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_all_to_treasury_when_share_is_max() {
        let split = split_protocol_fee(100, MAX_FEE_SHARE_BPS);
        assert_eq!(
            split,
            FeeSplitAmounts {
                treasury_amount: 100,
                bounty_amount: 0,
            }
        );
    }

    #[test]
    fn split_all_to_bounty_when_share_is_zero() {
        let split = split_protocol_fee(100, 0);
        assert_eq!(
            split,
            FeeSplitAmounts {
                treasury_amount: 0,
                bounty_amount: 100,
            }
        );
    }

    #[test]
    fn split_even_ratio_allocates_half_each() {
        let split = split_protocol_fee(100, 5_000);
        assert_eq!(
            split,
            FeeSplitAmounts {
                treasury_amount: 50,
                bounty_amount: 50,
            }
        );
    }

    #[test]
    fn split_remainder_goes_to_bounty_on_rounding() {
        let split = split_protocol_fee(10, 3_333);
        assert_eq!(split.treasury_amount, 3);
        assert_eq!(split.bounty_amount, 7);
        assert_eq!(split.treasury_amount + split.bounty_amount, 10);
    }

    #[test]
    fn split_zero_fee_yields_zeroes() {
        let split = split_protocol_fee(0, 7_500);
        assert_eq!(
            split,
            FeeSplitAmounts {
                treasury_amount: 0,
                bounty_amount: 0,
            }
        );
    }

    #[test]
    fn split_negative_fee_yields_zeroes() {
        let split = split_protocol_fee(-5, 5_000);
        assert_eq!(
            split,
            FeeSplitAmounts {
                treasury_amount: 0,
                bounty_amount: 0,
            }
        );
    }
}
