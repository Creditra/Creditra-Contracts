import re

try:
    with open('Cargo.lock', 'r', encoding='utf-8') as f: text = f.read()
except:
    with open('Cargo.lock', 'r', encoding='utf-16') as f: text = f.read()

blocks = text.split('[[package]]')
seen = set()
out = [blocks[0]]
for b in blocks[1:]:
    m = re.search(r'name\s*=\s*"([^"]+)"', b)
    if m:
        name = m.group(1)
        if name == 'creditra-collateral' and name in seen:
            continue
        seen.add(name)
    out.append(b)

with open('Cargo.lock', 'w', encoding='utf-8') as f:
    f.write('[[package]]'.join(out))
print('Fixed lockfile')
