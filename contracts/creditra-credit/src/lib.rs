pub mod accrual;
pub mod collateral;
pub mod contract;
pub mod error;
pub mod errors;
pub mod fees;
pub mod handshake;
/// Host-only per-entrypoint CPU-time instrumentation for regression baselines.
///
/// Requires the `instrument` Cargo feature; never compiled into the WASM binary.
#[cfg(all(not(target_arch = "wasm32"), feature = "instrument"))]
pub mod instrument;
pub mod key;
pub mod math_utils;
pub mod migrate;
pub mod msg;
pub mod oracles;
pub mod penalties;
pub mod state;
pub mod views;

pub use crate::error::ContractError;
pub use crate::migrate::{
    decode_contract_error, migrate_v1_error_encoding, ContractErrorEncodingV1,
    ContractErrorEncodingV2, ContractErrorKindV2, ErrorMigrationError,
};
