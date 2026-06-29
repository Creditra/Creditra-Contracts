//! Cross-chain liquidation hook.
//!
//! Consumes bridge attestations and triggers local liquidation safely.
//!
//! # `no_std` / WASM compatibility
//!
//! This module is **not** compiled into the WASM artifact. It is gated with
//! `#[cfg(not(target_arch = "wasm32"))]` because it uses `std` collections
//! (`HashSet`) and `println!` which are unavailable in the Soroban host
//! environment.  All production cross-chain logic that runs on-chain must go
//! through the main contract entrypoints; this module exists for off-chain
//! tooling and integration harnesses only.

#![cfg(not(target_arch = "wasm32"))]

use soroban_sdk::{contracttype, Bytes, String};

/// Bridge attestation coming from external chain
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BridgeAttestation {
    pub user: String,
    pub debt_amount: u128,
    pub source_chain: u64,
    pub nonce: u64,
    pub signature: Bytes,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CrossChainError {
    Unauthorized,
    InvalidSignature,
    ReplayAttack,
    InvalidAttestation,
}