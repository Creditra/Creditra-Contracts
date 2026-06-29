//! Cross-chain liquidation hook
//! Consumes bridge attestations and triggers local liquidation safely.

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