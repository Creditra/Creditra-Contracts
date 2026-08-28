import re

with open('contracts/credit/src/types.rs', 'r') as f:
    lines = f.read().split('\n')

start = lines.index('pub enum ContractError {') + 1
end = lines.index('}', start)

variants_lines = lines[start:end]
vals = []
for v in variants_lines:
    v = v.strip()
    if not v: continue
    m = re.search(r'=\s*(\d+)', v)
    if m:
        vals.append(int(m.group(1)))

print("Len:", len(vals))
print("Unique:", len(set(vals)))
