import re

with open('contracts/credit/src/types.rs', 'r') as f:
    lines = f.read().split('\n')

start = lines.index('pub enum ContractError {') + 1
end = lines.index('}', start)

variants = lines[start:end]
parsed = []
for v in variants:
    v = v.strip()
    if not v: continue
    m = re.match(r'^(\w+)\s*=\s*(\d+),?$', v)
    if m:
        parsed.append((int(m.group(2)), v))
    else:
        print("Unmatched:", v)

parsed.sort(key=lambda x: x[0])

sorted_variants = ["    " + p[1] + ("," if not p[1].endswith(",") else "") for p in parsed]

lines = lines[:start] + sorted_variants + lines[end:]

with open('contracts/credit/src/types.rs', 'w') as f:
    f.write('\n'.join(lines))
