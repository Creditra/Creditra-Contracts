import sys

with open('contracts/credit/src/types.rs', 'r') as f:
    text = f.read()

# Strip macro
text = text.replace("#[soroban_sdk::contracterror]", "")

# Delete extra brace
text = text.replace("    }\n}\n}\n\n/// Configuration", "    }\n}\n\n/// Configuration")

# Replace enum
old_enum = """    LiquidationGraceActive = 59,
    UnknownError = 60,
}"""
new_enum = """    LiquidationGraceActive = 59,
    UnknownError = 60,
    LimitDecreaseRequiresRepayment = 13,
    BorrowerBlocked = 16,
    LiquidityTokenCallFailed = 25,
    InsufficientRepaymentAllowance = 26,
    InsufficientRepaymentBalance = 27,
    BorrowerExposureCapExceeded = 43,
    InvalidAttestation = 53,
}"""
text = text.replace(old_enum, new_enum)

# Add to from_u32_safe
old_from = """            60 => Self::UnknownError,
            _ => Self::UnknownError,"""
new_from = """            60 => Self::UnknownError,
            13 => Self::LimitDecreaseRequiresRepayment,
            16 => Self::BorrowerBlocked,
            25 => Self::LiquidityTokenCallFailed,
            26 => Self::InsufficientRepaymentAllowance,
            27 => Self::InsufficientRepaymentBalance,
            43 => Self::BorrowerExposureCapExceeded,
            53 => Self::InvalidAttestation,
            _ => Self::UnknownError,"""
text = text.replace(old_from, new_from)

# Fix E0004
text = text.replace(
    "| Self::Overflow\n            | Self::LimitOutOfBounds",
    "| Self::Overflow\n            | Self::TimestampRegression\n            | Self::LimitOutOfBounds"
)
text = text.replace(
    "| Self::DrawReversalWindowExpired => Limit,",
    "| Self::DrawReversalWindowExpired\n            | Self::BorrowerExposureCapExceeded\n            | Self::LimitDecreaseRequiresRepayment => Limit,"
)
text = text.replace(
    "| Self::BountyNotSet => Liquidity,",
    "| Self::BountyNotSet\n            | Self::LiquidityTokenCallFailed\n            | Self::InsufficientRepaymentAllowance\n            | Self::InsufficientRepaymentBalance => Liquidity,"
)
text = text.replace(
    "| Self::BorrowerFrozen",
    "| Self::BorrowerFrozen\n            | Self::BorrowerBlocked"
)
text = text.replace(
    "| Self::AttestationBatchNotFound => Misc,",
    "| Self::AttestationBatchNotFound\n            | Self::InvalidAttestation => Misc,"
)

# Add From traits
impls = """
impl From<soroban_sdk::Error> for ContractError {
    fn from(err: soroban_sdk::Error) -> Self {
        if err.is_type(soroban_sdk::xdr::ScErrorType::Contract) {
            Self::from_u32_safe(err.get_code())
        } else {
            Self::UnknownError
        }
    }
}

impl<'a> From<&'a ContractError> for soroban_sdk::Error {
    fn from(err: &'a ContractError) -> Self {
        soroban_sdk::Error::from_contract_error(*err as u32)
    }
}

impl From<ContractError> for soroban_sdk::Error {
    fn from(err: ContractError) -> Self {
        soroban_sdk::Error::from_contract_error(err as u32)
    }
}
"""
text = text + impls

with open('contracts/credit/src/types.rs', 'w') as f:
    f.write(text)
