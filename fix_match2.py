import re

with open('contracts/credit/src/types.rs', 'r') as f:
    lines = f.read().split('\n')

start = lines.index('    pub fn from_u32_safe(code: u32) -> Self {') + 2
end = lines.index('            _ => Self::UnknownError,', start)

match_lines = lines[start:end]
parsed = []
for v in match_lines:
    v = v.strip()
    if not v: continue
    m = re.search(r'(\d+)\s*=>\s*Self::(\w+)', v)
    if m:
        parsed.append((int(m.group(1)), m.group(2)))

missing = [
    (13, 'LimitDecreaseRequiresRepayment'),
    (16, 'BorrowerBlocked'),
    (25, 'LiquidityTokenCallFailed'),
    (26, 'InsufficientRepaymentAllowance'),
    (27, 'InsufficientRepaymentBalance'),
    (43, 'BorrowerExposureCapExceeded'),
    (53, 'InvalidAttestation'),
    (60, 'UnknownError'),
    (54, 'RiskAdminCooldownActive'),
    (55, 'OracleNotFound'),
    (57, 'FreezeCooldownActive'),
    (58, 'AdminCollateralCooldownActive'),
    (59, 'LiquidationGraceActive'),
]
for p in missing:
    if p not in parsed:
        parsed.append(p)

unique_parsed = []
seen = set()
for p in parsed:
    if p[0] not in seen:
        seen.add(p[0])
        unique_parsed.append(p)
unique_parsed.sort(key=lambda x: x[0])

final_strings = []
for num, name in unique_parsed:
    final_strings.append(f"            {num} => Self::{name},")

lines = lines[:start] + final_strings + lines[end:]

with open('contracts/credit/src/types.rs', 'w') as f:
    f.write('\n'.join(lines))
