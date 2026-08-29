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
    m = re.search(r'^(\d+)\s*=>\s*Self::(\w+),', v)
    if m:
        parsed.append((int(m.group(1)), m.group(2)))
    else:
        print("Unmatched:", v)

parsed.sort(key=lambda x: x[0])

final_strings = []
seen = set()
for num, name in parsed:
    if num not in seen:
        seen.add(num)
        final_strings.append(f"            {num} => Self::{name},")

lines = lines[:start] + final_strings + lines[end:]

with open('contracts/credit/src/types.rs', 'w') as f:
    f.write('\n'.join(lines))
