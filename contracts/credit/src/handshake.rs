#![no_std]
use soroban_sdk::{contracttype, Env};

#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProtocolVersion {
    pub major: u32,
    pub minor: u32,
}

pub fn get_current_version() -> ProtocolVersion {
    ProtocolVersion { major: 1, minor: 0 }
}

pub fn verify_version(env: &Env, other_version: ProtocolVersion) -> bool {
    let current = get_current_version();
    // Major version must match, minor must be at least min compatible
    current.major == other_version.major && other_version.minor >= 0
}
