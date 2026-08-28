with open('contracts/credit/src/types.rs', 'r') as f:
    text = f.read()
text = text.replace("#[soroban_sdk::contracterror]", "")
old_enum = """    LiquidationGraceActive = 59,
}"""
new_enum = """    LiquidationGraceActive = 59,
    LimitDecreaseRequiresRepayment = 13,
    BorrowerBlocked = 16,
    LiquidityTokenCallFailed = 25,
    InsufficientRepaymentAllowance = 26,
    InsufficientRepaymentBalance = 27,
    BorrowerExposureCapExceeded = 43,
    UnknownError = 60,
}

impl From<ContractError> for soroban_sdk::Error {
    fn from(err: ContractError) -> Self {
        soroban_sdk::Error::from_contract_error(err as u32)
    }
}
"""
text = text.replace(old_enum, new_enum)
with open('contracts/credit/src/types.rs', 'w') as f:
    f.write(text)
