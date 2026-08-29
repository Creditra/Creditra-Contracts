with open('contracts/credit/src/types.rs', 'r') as f:
    text = f.read()

# ADD TO ENUM
text = text.replace(
    "    Overflow = 12,",
    "    Overflow = 12,\n    LimitDecreaseRequiresRepayment = 13,"
)

# ADD TO FROM_U32_SAFE
text = text.replace(
    "            12 => Self::Overflow,",
    "            12 => Self::Overflow,\n            13 => Self::LimitDecreaseRequiresRepayment,"
)

# ADD TO CATEGORY
text = text.replace(
    "| Self::RepayExceedsMaxAmount\n            | Self::DrawReversalWindowExpired => Limit,",
    "| Self::RepayExceedsMaxAmount\n            | Self::LimitDecreaseRequiresRepayment\n            | Self::DrawReversalWindowExpired => Limit,"
)

with open('contracts/credit/src/types.rs', 'w') as f:
    f.write(text)
