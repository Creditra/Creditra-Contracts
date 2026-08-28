import re
with open('contracts/credit/src/types.rs', 'r') as f:
    text = f.read()

text = re.sub(r'\}\n\s*\}\n\s*\}\n\s*\}\n\s*/// Configuration', '}\n}\n\n/// Configuration', text)
with open('contracts/credit/src/types.rs', 'w') as f:
    f.write(text)
