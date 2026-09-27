//! Backward-compatible `ContractError` wire-format migration helpers.
//!
//! V1 used Serde's externally tagged enum representation. V2 uses a
//! versioned envelope and an internally tagged error value so clients can
//! inspect the encoding version before decoding variant-specific fields.

use cosmwasm_std::{from_json, to_json_vec};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Wire-format version emitted by [`ContractErrorEncodingV2`].
pub const CONTRACT_ERROR_ENCODING_V2: u8 = 2;

/// Legacy V1 JSON representation.
///
/// This type deliberately preserves Serde's default externally tagged enum
/// encoding. Existing bytes such as `"Unauthorized"`,
/// `{"CreditLineNotFound":7}`, and `{"DrawNotFound":[3,7]}` therefore remain
/// decodable during the migration window.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum ContractErrorEncodingV1 {
    /// A standard-library error, encoded as its display message.
    Std(String),
    /// The requested credit line does not exist.
    CreditLineNotFound(u64),
    /// The requested draw does not exist on the supplied credit line.
    DrawNotFound(u64, u64),
    /// The caller is not authorized.
    Unauthorized,
    /// Available collateral cannot cover the operation.
    CollateralInsufficient,
    /// The collateral balance is below the requested withdrawal.
    InsufficientCollateralBalance,
    /// The requested amount is invalid.
    InvalidAmount,
    /// The liquidation settlement was already processed.
    AlreadySettled,
    /// The supplied oracle price is invalid.
    OraclePriceInvalid,
    /// The configured oracle quorum was not met.
    OracleQuorumNotMet,
}

/// Stable V2 error payload.
///
/// The `code` field is always present. Variant data, when required, is stored
/// under `details`; unit variants omit `details`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "code", content = "details", rename_all = "snake_case")]
pub enum ContractErrorKindV2 {
    /// A standard-library error, encoded as its display message.
    Std(String),
    /// The requested credit line does not exist.
    CreditLineNotFound(u64),
    /// The requested draw does not exist on the supplied credit line.
    DrawNotFound {
        /// Draw identifier.
        draw_id: u64,
        /// Credit-line identifier.
        credit_line_id: u64,
    },
    /// The caller is not authorized.
    Unauthorized,
    /// Available collateral cannot cover the operation.
    CollateralInsufficient,
    /// The collateral balance is below the requested withdrawal.
    InsufficientCollateralBalance,
    /// The requested amount is invalid.
    InvalidAmount,
    /// The liquidation settlement was already processed.
    AlreadySettled,
    /// The supplied oracle price is invalid.
    OraclePriceInvalid,
    /// The configured oracle quorum was not met.
    OracleQuorumNotMet,
}

/// Versioned V2 `ContractError` envelope.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContractErrorEncodingV2 {
    /// Encoding version. This must be [`CONTRACT_ERROR_ENCODING_V2`].
    pub version: u8,
    /// Stable, tagged error payload.
    pub error: ContractErrorKindV2,
}

/// Failure returned while decoding or migrating a contract error.
#[derive(Debug, Error)]
pub enum ErrorMigrationError {
    /// Input was neither a supported V1 value nor a valid V2 envelope.
    #[error("invalid ContractError encoding: {0}")]
    InvalidEncoding(String),
    /// A versioned envelope used an unsupported version.
    #[error("unsupported ContractError encoding version {0}")]
    UnsupportedVersion(u8),
    /// V2 serialization failed.
    #[error("failed to encode V2 ContractError: {0}")]
    Encode(String),
}

impl From<ContractErrorEncodingV1> for ContractErrorKindV2 {
    fn from(value: ContractErrorEncodingV1) -> Self {
        match value {
            ContractErrorEncodingV1::Std(message) => Self::Std(message),
            ContractErrorEncodingV1::CreditLineNotFound(id) => Self::CreditLineNotFound(id),
            ContractErrorEncodingV1::DrawNotFound(draw_id, credit_line_id) => Self::DrawNotFound {
                draw_id,
                credit_line_id,
            },
            ContractErrorEncodingV1::Unauthorized => Self::Unauthorized,
            ContractErrorEncodingV1::CollateralInsufficient => Self::CollateralInsufficient,
            ContractErrorEncodingV1::InsufficientCollateralBalance => {
                Self::InsufficientCollateralBalance
            }
            ContractErrorEncodingV1::InvalidAmount => Self::InvalidAmount,
            ContractErrorEncodingV1::AlreadySettled => Self::AlreadySettled,
            ContractErrorEncodingV1::OraclePriceInvalid => Self::OraclePriceInvalid,
            ContractErrorEncodingV1::OracleQuorumNotMet => Self::OracleQuorumNotMet,
        }
    }
}

impl From<ContractErrorEncodingV1> for ContractErrorEncodingV2 {
    fn from(value: ContractErrorEncodingV1) -> Self {
        Self {
            version: CONTRACT_ERROR_ENCODING_V2,
            error: value.into(),
        }
    }
}

/// Convert serialized V1 JSON bytes into the V2 wire format.
///
/// This helper is pure and does not mutate contract state. Invalid, truncated,
/// or already-versioned input returns a typed error instead of panicking.
pub fn migrate_v1_error_encoding(input: &[u8]) -> Result<Vec<u8>, ErrorMigrationError> {
    let legacy = from_json::<ContractErrorEncodingV1>(input)
        .map_err(|error| ErrorMigrationError::InvalidEncoding(error.to_string()))?;
    let migrated = ContractErrorEncodingV2::from(legacy);
    to_json_vec(&migrated).map_err(|error| ErrorMigrationError::Encode(error.to_string()))
}

/// Decode either a legacy V1 value or a V2 envelope into the V2 model.
///
/// Clients can use this during a rolling upgrade: V1 responses are normalized
/// in memory, while V2 responses are returned unchanged after version
/// validation.
pub fn decode_contract_error(input: &[u8]) -> Result<ContractErrorEncodingV2, ErrorMigrationError> {
    if let Ok(versioned) = from_json::<ContractErrorEncodingV2>(input) {
        if versioned.version != CONTRACT_ERROR_ENCODING_V2 {
            return Err(ErrorMigrationError::UnsupportedVersion(versioned.version));
        }
        return Ok(versioned);
    }

    from_json::<ContractErrorEncodingV1>(input)
        .map(ContractErrorEncodingV2::from)
        .map_err(|error| ErrorMigrationError::InvalidEncoding(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{
        decode_contract_error, migrate_v1_error_encoding, ContractErrorEncodingV1,
        ContractErrorEncodingV2, ContractErrorKindV2, ErrorMigrationError,
        CONTRACT_ERROR_ENCODING_V2,
    };
    use cosmwasm_std::{from_json, to_json_vec};

    #[test]
    fn migrates_every_v1_variant_without_losing_details() {
        let cases = [
            (
                ContractErrorEncodingV1::Std("storage failure".to_owned()),
                ContractErrorKindV2::Std("storage failure".to_owned()),
            ),
            (
                ContractErrorEncodingV1::CreditLineNotFound(42),
                ContractErrorKindV2::CreditLineNotFound(42),
            ),
            (
                ContractErrorEncodingV1::DrawNotFound(7, 42),
                ContractErrorKindV2::DrawNotFound {
                    draw_id: 7,
                    credit_line_id: 42,
                },
            ),
            (
                ContractErrorEncodingV1::Unauthorized,
                ContractErrorKindV2::Unauthorized,
            ),
            (
                ContractErrorEncodingV1::CollateralInsufficient,
                ContractErrorKindV2::CollateralInsufficient,
            ),
            (
                ContractErrorEncodingV1::InsufficientCollateralBalance,
                ContractErrorKindV2::InsufficientCollateralBalance,
            ),
            (
                ContractErrorEncodingV1::InvalidAmount,
                ContractErrorKindV2::InvalidAmount,
            ),
            (
                ContractErrorEncodingV1::AlreadySettled,
                ContractErrorKindV2::AlreadySettled,
            ),
            (
                ContractErrorEncodingV1::OraclePriceInvalid,
                ContractErrorKindV2::OraclePriceInvalid,
            ),
            (
                ContractErrorEncodingV1::OracleQuorumNotMet,
                ContractErrorKindV2::OracleQuorumNotMet,
            ),
        ];

        for (legacy, expected) in cases {
            let input = to_json_vec(&legacy).expect("test fixture must serialize");
            let output = migrate_v1_error_encoding(&input).expect("valid V1 fixture must migrate");
            let decoded: ContractErrorEncodingV2 =
                from_json(output).expect("migration must emit valid V2 JSON");
            assert_eq!(decoded.version, CONTRACT_ERROR_ENCODING_V2);
            assert_eq!(decoded.error, expected);
        }
    }

    #[test]
    fn preserves_known_v1_wire_shapes() {
        let unit =
            decode_contract_error(br#""Unauthorized""#).expect("legacy unit variant must decode");
        assert_eq!(unit.error, ContractErrorKindV2::Unauthorized);

        let one_field = decode_contract_error(br#"{"CreditLineNotFound":9}"#)
            .expect("legacy newtype variant must decode");
        assert_eq!(one_field.error, ContractErrorKindV2::CreditLineNotFound(9));

        let tuple = decode_contract_error(br#"{"DrawNotFound":[4,9]}"#)
            .expect("legacy tuple variant must decode");
        assert_eq!(
            tuple.error,
            ContractErrorKindV2::DrawNotFound {
                draw_id: 4,
                credit_line_id: 9,
            }
        );
    }

    #[test]
    fn decoder_accepts_v2_without_reencoding() {
        let expected = ContractErrorEncodingV2 {
            version: CONTRACT_ERROR_ENCODING_V2,
            error: ContractErrorKindV2::InvalidAmount,
        };
        let bytes = to_json_vec(&expected).expect("test fixture must serialize");
        assert_eq!(
            decode_contract_error(&bytes).expect("valid V2 fixture must decode"),
            expected
        );
    }

    #[test]
    fn rejects_unsupported_v2_version() {
        let input = br#"{"version":3,"error":{"code":"unauthorized"}}"#;
        assert!(matches!(
            decode_contract_error(input),
            Err(ErrorMigrationError::UnsupportedVersion(3))
        ));
    }

    #[test]
    fn rejects_unknown_variants_and_malformed_input() {
        for input in [
            br#""FutureError""#.as_slice(),
            br#"{"DrawNotFound":[1]}"#.as_slice(),
            br#"{"CreditLineNotFound":-1}"#.as_slice(),
            br#"not-json"#.as_slice(),
            b"".as_slice(),
        ] {
            assert!(matches!(
                decode_contract_error(input),
                Err(ErrorMigrationError::InvalidEncoding(_))
            ));
        }
    }

    #[test]
    fn v1_only_migrator_rejects_v2_input() {
        let input = br#"{"version":2,"error":{"code":"unauthorized"}}"#;
        assert!(matches!(
            migrate_v1_error_encoding(input),
            Err(ErrorMigrationError::InvalidEncoding(_))
        ));
    }
}
