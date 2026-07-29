// SPDX-License-Identifier: MIT

//! Focused tests for the freeze (v7) `capabilities()` view.
//!
//! # What
//!
//! Verifies the read-only [`freeze_capabilities`] view returns the correct
//! `u64` bitmask of supported freeze features. Tests are organized into
//! three sections:
//!
//! 1. **Free-function direct call** — calls [`freeze_capabilities`] without
//!    deploying any contract to confirm the return value is a compile-time
//!    constant that requires no on-chain state.
//!
//! 2. **Individual bit assertions** — checks every named `CAPABILITY_*`
//!    constant is set in the returned mask, and that no bits outside the
//!    defined set are unexpectedly lit.
//!
//! 3. **Consistency / cross-check** — determinism, non-zero mask, and mask
//!    equality with [`ALL_FREEZE_CAPABILITIES`].
//!
//! # See also
//! - [`creditra_freeze::views`] — the implementation under test.
//! - [`contracts/collateral/tests/views_capabilities.rs`] — canonical test
//!   pattern this file follows.

use creditra_freeze::views::{
    freeze_capabilities, ALL_FREEZE_CAPABILITIES, CAPABILITY_BORROWER_EXPIRY,
    CAPABILITY_FREEZE_BORROWER, CAPABILITY_FREEZE_COOLDOWN, CAPABILITY_FREEZE_CREDIT_LINE,
    CAPABILITY_FREEZE_DRAWS, CAPABILITY_FREEZE_REASON,
};
use soroban_sdk::Env;

// ═══════════════════════════════════════════════════════════════════════════
// Section 1 — Free-function direct call (no contract deploy needed)
// ═══════════════════════════════════════════════════════════════════════════

/// Calling the free function directly returns [`ALL_FREEZE_CAPABILITIES`].
#[test]
fn freeze_capabilities_direct_call_returns_all_capabilities() {
    let env = Env::default();
    let caps = freeze_capabilities(&env);

    assert_eq!(
        caps,
        ALL_FREEZE_CAPABILITIES,
        "freeze_capabilities() must equal ALL_FREEZE_CAPABILITIES"
    );
}

/// The aggregate mask must be non-zero (guards against a silent zeroing).
#[test]
fn freeze_capabilities_direct_call_is_non_zero() {
    let env = Env::default();
    let caps = freeze_capabilities(&env);

    assert_ne!(caps, 0, "ALL_FREEZE_CAPABILITIES must not be zero");
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 2 — Individual bit assertions
// ═══════════════════════════════════════════════════════════════════════════

/// Bit 0 — global draws freeze / unfreeze (`freeze_draws`, `unfreeze_draws`).
#[test]
fn freeze_capabilities_includes_freeze_draws_bit() {
    let env = Env::default();
    let caps = freeze_capabilities(&env);

    assert_eq!(
        caps & CAPABILITY_FREEZE_DRAWS,
        CAPABILITY_FREEZE_DRAWS,
        "CAPABILITY_FREEZE_DRAWS (bit 0) must be set"
    );
}

/// Bit 1 — per-borrower credit-line freeze (`freeze_credit_line`, `unfreeze_credit_line`).
#[test]
fn freeze_capabilities_includes_freeze_credit_line_bit() {
    let env = Env::default();
    let caps = freeze_capabilities(&env);

    assert_eq!(
        caps & CAPABILITY_FREEZE_CREDIT_LINE,
        CAPABILITY_FREEZE_CREDIT_LINE,
        "CAPABILITY_FREEZE_CREDIT_LINE (bit 1) must be set"
    );
}

/// Bit 2 — time-bounded borrower freeze (`freeze_borrower_until`, `unfreeze_borrower`).
#[test]
fn freeze_capabilities_includes_freeze_borrower_bit() {
    let env = Env::default();
    let caps = freeze_capabilities(&env);

    assert_eq!(
        caps & CAPABILITY_FREEZE_BORROWER,
        CAPABILITY_FREEZE_BORROWER,
        "CAPABILITY_FREEZE_BORROWER (bit 2) must be set"
    );
}

/// Bit 3 — structured `FreezeReason` classification and reason queries.
#[test]
fn freeze_capabilities_includes_freeze_reason_bit() {
    let env = Env::default();
    let caps = freeze_capabilities(&env);

    assert_eq!(
        caps & CAPABILITY_FREEZE_REASON,
        CAPABILITY_FREEZE_REASON,
        "CAPABILITY_FREEZE_REASON (bit 3) must be set"
    );
}

/// Bit 4 — `get_borrower_frozen_until` expiry query.
#[test]
fn freeze_capabilities_includes_borrower_expiry_bit() {
    let env = Env::default();
    let caps = freeze_capabilities(&env);

    assert_eq!(
        caps & CAPABILITY_BORROWER_EXPIRY,
        CAPABILITY_BORROWER_EXPIRY,
        "CAPABILITY_BORROWER_EXPIRY (bit 4) must be set"
    );
}

/// Bit 5 — admin cool-off guard on state-changing freeze operations.
#[test]
fn freeze_capabilities_includes_freeze_cooldown_bit() {
    let env = Env::default();
    let caps = freeze_capabilities(&env);

    assert_eq!(
        caps & CAPABILITY_FREEZE_COOLDOWN,
        CAPABILITY_FREEZE_COOLDOWN,
        "CAPABILITY_FREEZE_COOLDOWN (bit 5) must be set"
    );
}

/// Verify all 6 capability bits are set by testing the expected full mask.
#[test]
fn freeze_capabilities_all_six_bits_set() {
    let env = Env::default();
    let caps = freeze_capabilities(&env);

    let expected: u64 = (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3) | (1 << 4) | (1 << 5);

    assert_eq!(
        caps, expected,
        "freeze_capabilities() must have exactly 6 bits set (bits 0-5)"
    );
}

/// No bits outside the defined 6-bit range (bits 0–5) should be set.
///
/// This guards against accidentally advertising capabilities that are not yet
/// implemented or that have drifted from the constant definitions.
#[test]
fn freeze_capabilities_no_undefined_bits_set() {
    let env = Env::default();
    let caps = freeze_capabilities(&env);

    let defined_mask: u64 = CAPABILITY_FREEZE_DRAWS
        | CAPABILITY_FREEZE_CREDIT_LINE
        | CAPABILITY_FREEZE_BORROWER
        | CAPABILITY_FREEZE_REASON
        | CAPABILITY_BORROWER_EXPIRY
        | CAPABILITY_FREEZE_COOLDOWN;

    assert_eq!(
        caps & !defined_mask,
        0,
        "No undefined bits must be set in freeze_capabilities()"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Section 3 — Consistency / cross-check
// ═══════════════════════════════════════════════════════════════════════════

/// Two successive calls return the same value (pure, no hidden state).
#[test]
fn freeze_capabilities_deterministic_same_result_twice() {
    let env = Env::default();

    let caps1 = freeze_capabilities(&env);
    let caps2 = freeze_capabilities(&env);

    assert_eq!(
        caps1, caps2,
        "freeze_capabilities() must be deterministic"
    );
}

/// The returned value equals the exported [`ALL_FREEZE_CAPABILITIES`] constant.
#[test]
fn freeze_capabilities_equals_all_freeze_capabilities_constant() {
    let env = Env::default();
    let caps = freeze_capabilities(&env);

    assert_eq!(
        caps,
        ALL_FREEZE_CAPABILITIES,
        "freeze_capabilities() must equal the ALL_FREEZE_CAPABILITIES constant"
    );
}

/// Each individual `CAPABILITY_*` constant has exactly one bit set.
///
/// Guards against accidental multi-bit constants that would cause incorrect
/// feature-detection logic in clients using bitwise tests.
#[test]
fn each_capability_constant_is_a_single_bit() {
    let constants = [
        CAPABILITY_FREEZE_DRAWS,
        CAPABILITY_FREEZE_CREDIT_LINE,
        CAPABILITY_FREEZE_BORROWER,
        CAPABILITY_FREEZE_REASON,
        CAPABILITY_BORROWER_EXPIRY,
        CAPABILITY_FREEZE_COOLDOWN,
    ];

    for (idx, &cap) in constants.iter().enumerate() {
        assert_ne!(cap, 0, "Capability constant at index {} must not be zero", idx);
        assert_eq!(
            cap & (cap - 1),
            0,
            "Capability constant at index {} (value {:#x}) must have exactly one bit set",
            idx,
            cap
        );
    }
}

/// All individual `CAPABILITY_*` constants are distinct (no two share a bit).
#[test]
fn all_capability_constants_are_distinct() {
    let constants = [
        CAPABILITY_FREEZE_DRAWS,
        CAPABILITY_FREEZE_CREDIT_LINE,
        CAPABILITY_FREEZE_BORROWER,
        CAPABILITY_FREEZE_REASON,
        CAPABILITY_BORROWER_EXPIRY,
        CAPABILITY_FREEZE_COOLDOWN,
    ];

    for i in 0..constants.len() {
        for j in (i + 1)..constants.len() {
            assert_eq!(
                constants[i] & constants[j],
                0,
                "Capability constants at indices {} and {} must not share any bits \
                 ({:#x} & {:#x} = {:#x})",
                i,
                j,
                constants[i],
                constants[j],
                constants[i] & constants[j]
            );
        }
    }
}

/// Pin the total number of defined capabilities. Update this constant only
/// when adding a new capability bit at the end of the list.
#[test]
fn capability_count_is_known() {
    const EXPECTED_CAPABILITY_COUNT: usize = 6;

    let constants = [
        CAPABILITY_FREEZE_DRAWS,
        CAPABILITY_FREEZE_CREDIT_LINE,
        CAPABILITY_FREEZE_BORROWER,
        CAPABILITY_FREEZE_REASON,
        CAPABILITY_BORROWER_EXPIRY,
        CAPABILITY_FREEZE_COOLDOWN,
    ];

    assert_eq!(
        constants.len(),
        EXPECTED_CAPABILITY_COUNT,
        "Capability count changed — update EXPECTED_CAPABILITY_COUNT"
    );
}
