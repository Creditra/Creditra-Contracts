with open('contracts/credit/src/types.rs', 'r') as f:
    text = f.read()

# Fix the duplicate braces at the end of `category` method
text = text.replace("    }\n}\n}\n}\n\n/// Configuration", "    }\n}\n\n/// Configuration")

# 1. Add variants to ContractError enum
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

# 2. Add to from_u32_safe
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

# 3. Add to category

old_block = "            | Self::FreezeCooldownActive => Block,"
new_block = "            | Self::FreezeCooldownActive\n            | Self::BorrowerBlocked => Block,"
text = text.replace(old_block, new_block)

old_limit = "            | Self::DrawReversalWindowExpired => Limit,"
new_limit = "            | Self::DrawReversalWindowExpired\n            | Self::BorrowerExposureCapExceeded\n            | Self::LimitDecreaseRequiresRepayment => Limit,"
text = text.replace(old_limit, new_limit)

old_liq = "            | Self::BountyNotSet => Liquidity,"
new_liq = "            | Self::BountyNotSet\n            | Self::LiquidityTokenCallFailed\n            | Self::InsufficientRepaymentAllowance\n            | Self::InsufficientRepaymentBalance => Liquidity,"
text = text.replace(old_liq, new_liq)

old_misc = "            | Self::AttestationBatchNotFound => Misc,"
new_misc = "            | Self::AttestationBatchNotFound\n            | Self::UnknownError => Misc,"
text = text.replace(old_misc, new_misc)


with open('contracts/credit/src/types.rs', 'w') as f:
    f.write(text)
