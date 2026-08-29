with open('contracts/credit/src/types.rs', 'r') as f:
    text = f.read()

from_u32 = """
    pub fn from_u32_safe(code: u32) -> Self {
        match code {
            1 => Self::Unauthorized,
            2 => Self::NotAdmin,
            3 => Self::CreditLineNotFound,
            4 => Self::CreditLineClosed,
            5 => Self::InvalidAmount,
            _ => Self::NotAdmin,
        }
    }
"""
text = text.replace(
    "    pub fn category(&self) -> ContractErrorCategory {",
    from_u32 + "    pub fn category(&self) -> ContractErrorCategory {"
)
with open('contracts/credit/src/types.rs', 'w') as f:
    f.write(text)
