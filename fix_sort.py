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
    m = re.search(r'(\w+)\s*=\s*(\d+),', v)
    if m: parsed.append((int(m.group(2)), m.group(1)))
parsed.sort(key=lambda x: x[0])
final_strings = []
for num, name in parsed:
    final_strings.append(f"    {name} = {num},")
lines = lines[:start] + final_strings + lines[end:]
with open('contracts/credit/src/types.rs', 'w') as f:
    f.write('\n'.join(lines))
