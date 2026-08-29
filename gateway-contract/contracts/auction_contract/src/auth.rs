//! Auction bidder / winner authorization helpers.
//!
//! # What
//!
//! Tiny helpers that cryptographically bind the party whose bidder-owned
//! state is being mutated by an auction entrypoint:
//!
//! - [`require_bidder_auth`] — used by the bid-placing mutation
//!   ([`crate::Auction::place_bid`]).
//! - [`require_winner_auth`] — used by the claim mutation
//!   ([`crate::Auction::claim_auction`]).
//!
//! Both delegate to the Soroban host's authorization framework via
//! [`Address::require_auth`], which only returns when the transaction is
//! signed (or auth-entry attested) by the supplied address.
//!
//! # How
//!
//! Each helper is a single call: `require_auth(&env, &address)`. They exist
//! so that every *bidder-facing* auction mutation reads exactly one line to
//! enforce the invariant and so the invariant is documented in one place.
//!
//! # The invariant
//!
//! > **Every auction mutation that changes bidder-owned state must verify
//! > the affected address's authorization.**
//!
//! | Mutation                    | Authorized party          | Helper                |
//! |-----------------------------|---------------------------|-----------------------|
//! | `place_bid`                 | The bidder                | [`require_bidder_auth`] |
//! | `claim_auction`             | The recorded winner       | [`require_winner_auth`] |
//! | `init_auction` / `close_auction` / `settle_default_liquidation` / `set_liquidation_grace_window` | The factory contract | (see storage factory gating) |
//!
//! Factory-gated mutations are not bidder mutations; they mutate protocol
//! lifecycle state and are gated by the registered factory contract's
//! authorization instead. Bidder-facing mutations (`place_bid`,
//! `claim_auction`) are the only ones these helpers cover.
//!
//! # Why
//!
//! Concentrating the two bidder auth checks here makes it mechanically
//! impossible to add a new bidder-facing mutation that forgets to verify the
//! caller's ownership of the affected bidder/winner address. It also keeps
//! the [`crate::Auction`] impl's auth surface auditable in one small module
//! (mirroring `contracts/credit/src/auth.rs`).
//!
//! # See also
//!
//! - [`docs/threat-model.md`](../../../docs/threat-model.md)
//! - [`crate::storage::get_factory_contract`] for the factory gating used by
//!   the lifecycle mutations.

use soroban_sdk::Address;

/// Require the `bidder`'s authorization for a bid-placing mutation.
///
/// Binds the `bidder` argument of [`crate::Auction::place_bid`] to a
/// cryptographic attestation. Only a caller able to satisfy
/// [`Address::require_auth`] for `bidder` may mutate the auction's
/// highest-bidder state on their behalf.
pub fn require_bidder_auth(bidder: &Address) {
    bidder.require_auth();
}

/// Require the `winner`'s authorization for a claim mutation.
///
/// Binds the recorded winner of [`crate::Auction::claim_auction`] to a
/// cryptographic attestation. Only the winner may move the auction's
/// recovered proceeds out of the contract.
pub fn require_winner_auth(winner: &Address) {
    winner.require_auth();
}
