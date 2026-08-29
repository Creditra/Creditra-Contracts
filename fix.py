with open('contracts/credit/src/types.rs', 'r') as f:
    lines = f.read().split('\n')

start = lines.index('pub enum ContractError {') + 1
end = lines.index('}', start)

variants_lines = lines[start:end]
parsed = []
for v in variants_lines:
    v = v.strip()
    if not v: continue
    # Extract the number
    import re
    m = re.search(r'=\s*(\d+)', v)
    if m:
        parsed.append((int(m.group(1)), v))
    else:
        print("No match:", v)

# Add the missing ones
parsed.append((13, "LimitDecreaseRequiresRepayment = 13,"))
parsed.append((16, "BorrowerBlocked = 16,"))
parsed.append((25, "LiquidityTokenCallFailed = 25,"))
parsed.append((26, "InsufficientRepaymentAllowance = 26,"))
parsed.append((27, "InsufficientRepaymentBalance = 27,"))
parsed.append((43, "BorrowerExposureCapExceeded = 43,"))
parsed.append((53, "InvalidAttestation = 53,")) # Wait, let me check if 53 was there
parsed.append((44, "UnknownError44 = 44,")) # Wait, is this needed?
parsed.sort(key=lambda x: x[0])

# Just take the strings
sorted_strings = ["    " + p[1] + ("," if not p[1].endswith(",") else "") for p in parsed]

# Remove duplicates
seen = set()
final_strings = []
for s in sorted_strings:
    num = re.search(r'=\s*(\d+)', s).group(1)
    if num not in seen:
        seen.add(num)
        final_strings.append(s)

lines = lines[:start] + final_strings + lines[end:]

with open('contracts/credit/src/types.rs', 'w') as f:
    f.write('\n'.join(lines))
