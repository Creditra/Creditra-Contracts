with open('contracts/credit/src/types.rs', 'r') as f:
    text = f.read()
text = text.replace("    }\n}\n}\n\n/// Configuration", "    }\n}\n\n/// Configuration")
text = text.replace(
    "| Self::Overflow\n            | Self::LimitOutOfBounds",
    "| Self::Overflow\n            | Self::TimestampRegression\n            | Self::LimitOutOfBounds"
)
with open('contracts/credit/src/types.rs', 'w') as f:
    f.write(text)
