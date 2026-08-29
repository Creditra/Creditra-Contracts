with open('contracts/credit/src/types.rs', 'r') as f:
    text = f.read()

text = text.replace(
    "| Self::AttestationBatchNotFound\n            | Self::UnknownError => Misc,",
    "| Self::AttestationBatchNotFound\n            | Self::InvalidAttestation\n            | Self::UnknownError => Misc,"
)

with open('contracts/credit/src/types.rs', 'w') as f:
    f.write(text)
