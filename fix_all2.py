import re

with open('contracts/credit/src/types.rs', 'r') as f:
    lines = f.read().split('\n')

start = lines.index('pub enum ContractError {') + 1
end = lines.index('}', start)

variants_lines = lines[start:end]
parsed = []
for v in variants_lines:
    v = v.strip()
    if not v: continue
    m = re.search(r'(\w+)\s*=\s*(\d+)', v)
    if m:
        parsed.append((int(m.group(2)), m.group(1)))

missing = [
    (13, 'LimitDecreaseRequiresRepayment'),
    (16, 'BorrowerBlocked'),
    (25, 'LiquidityTokenCallFailed'),
    (26, 'InsufficientRepaymentAllowance'),
    (27, 'InsufficientRepaymentBalance'),
    (43, 'BorrowerExposureCapExceeded'),
    (53, 'InvalidAttestation'),
    (60, 'UnknownError')
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
    final_strings.append(f"    {name} = {num},")

lines = lines[:start] + final_strings + lines[end:]

with open('contracts/credit/src/types.rs', 'w') as f:
    f.write('\n'.join(lines))
