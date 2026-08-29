with open('contracts/credit/src/types.rs', 'r') as f:
    text = f.read()

import re
text = re.sub(r'\}\n\s*\}\n\s*\}\n\s*\}\n\s*/// Configuration', r'}\n}\n\n/// Configuration', text)

with open('contracts/credit/src/types.rs', 'w') as f:
    f.write(text)
