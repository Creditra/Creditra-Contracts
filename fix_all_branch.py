import re

with open('contracts/credit/src/types.rs', 'r') as f:
    text = f.read()

# Fix ContractError Enum
old_enum = "    LiquidationGraceActive = 59,\n}"
new_enum = """    LiquidationGraceActive = 59,
    BorrowerBlocked = 16,
    LiquidityTokenCallFailed = 25,
    InsufficientRepaymentAllowance = 26,
    InsufficientRepaymentBalance = 27,
    BorrowerExposureCapExceeded = 43,
    LimitDecreaseRequiresRepayment = 13,
    UnknownError = 60,
}"""
text = text.replace(old_enum, new_enum)

# Fix from_u32_safe match
old_match = "            59 => Self::LiquidationGraceActive,\n            _ => Self::UnknownError,"
new_match = """            59 => Self::LiquidationGraceActive,
            16 => Self::BorrowerBlocked,
            25 => Self::LiquidityTokenCallFailed,
            26 => Self::InsufficientRepaymentAllowance,
            27 => Self::InsufficientRepaymentBalance,
            43 => Self::BorrowerExposureCapExceeded,
            13 => Self::LimitDecreaseRequiresRepayment,
            60 => Self::UnknownError,
            _ => Self::UnknownError,"""
text = text.replace(old_match, new_match)

# Fix category match arms
text = text.replace("            | Self::FreezeCooldownActive => Block,", "            | Self::FreezeCooldownActive\n            | Self::BorrowerBlocked => Block,")
text = text.replace("            | Self::DrawReversalWindowExpired => Limit,", "            | Self::DrawReversalWindowExpired\n            | Self::BorrowerExposureCapExceeded\n            | Self::LimitDecreaseRequiresRepayment => Limit,")
text = text.replace("            | Self::BountyNotSet => Liquidity,", "            | Self::BountyNotSet\n            | Self::LiquidityTokenCallFailed\n            | Self::InsufficientRepaymentAllowance\n            | Self::InsufficientRepaymentBalance => Liquidity,")
text = text.replace("            | Self::AttestationBatchNotFound => Misc,", "            | Self::AttestationBatchNotFound\n            | Self::UnknownError\n            | Self::InvalidAttestation => Misc,")

with open('contracts/credit/src/types.rs', 'w') as f:
    f.write(text)
