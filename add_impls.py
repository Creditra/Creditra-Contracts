with open('contracts/credit/src/types.rs', 'r') as f:
    text = f.read()

impls = """
impl From<soroban_sdk::Error> for ContractError {
    fn from(err: soroban_sdk::Error) -> Self {
        // Assume all soroban_sdk::Error map to UnknownError if we have to decode them,
        // or actually try to parse. We can use our from_u32_safe!
        // But from_u32_safe is not written yet in this snippet, let's just use UnknownError.
        Self::UnknownError
    }
}

impl<'a> From<&'a ContractError> for soroban_sdk::Error {
    fn from(err: &'a ContractError) -> Self {
        soroban_sdk::Error::from_contract_error(*err as u32)
    }
}
"""
text = text + impls
with open('contracts/credit/src/types.rs', 'w') as f:
    f.write(text)
