#!/usr/bin/env python3
"""Print every ContractError variant declared in contracts/credit/src/types.rs.

The script is intentionally dependency-free; it parses the file with a simple
regex rather than running rustc. It is meant as a quick reference for
indexer/SDK authors who need to keep an error-code table in sync.
"""

from __future__ import annotations

import sys
import re
import pathlib

REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent
TYPES_RS = REPO_ROOT / "contracts" / "credit" / "src" / "types.rs"

def parse_enum(source: str, enum_name: str) -> list[tuple[int, str, str]]:
    """Returns list of (code, name, description)."""
    enum_open = re.search(rf"pub\s+enum\s+{enum_name}\s*\{{", source)
    if not enum_open:
        raise SystemExit(f"{enum_name} enum not found in types.rs")
    
    body_start = enum_open.end()
    depth = 1
    i = body_start
    while i < len(source) and depth > 0:
        ch = source[i]
        if ch == "{": depth += 1
        elif ch == "}": depth -= 1
        i += 1
    
    body = source[body_start : i - 1]
    
    variants = []
    # Parse lines. A variant might be preceded by `///` comments.
    desc_lines = []
    for line in body.splitlines():
        line = line.strip()
        if line.startswith("///"):
            desc = line.lstrip("/").strip()
            if desc:
                desc_lines.append(desc)
        elif line.startswith("//"):
            pass
        else:
            m = re.match(r"^([A-Za-z][A-Za-z0-9]*)\s*=\s*(\d+)\s*,", line)
            if m:
                name = m.group(1)
                code = int(m.group(2))
                desc = " ".join(desc_lines) if desc_lines else ""
                variants.append((code, name, desc))
                desc_lines = []
            else:
                if not line:
                    desc_lines = []
    
    variants.sort()
    return variants

def extract_category_mapping(source: str) -> dict[str, str]:
    """Extract variant -> category name from category()."""
    fn_start = re.search(r"pub fn category\s*\(&self\)\s*->\s*ContractErrorCategory\s*\{", source)
    if not fn_start:
        raise SystemExit("category() method not found in types.rs")
    
    i = fn_start.end()
    depth = 1
    while i < len(source) and depth > 0:
        if source[i] == "{": depth += 1
        elif source[i] == "}": depth -= 1
        i += 1
    body = source[fn_start.end() : i - 1]
    
    mapping = {}
    arm_re = re.compile(r"(?P<variants>Self::\w+(?:\s*\|\s*Self::\w+)*)\s*=>\s*\{?\s*ContractErrorCategory::(?P<cat>\w+)", re.DOTALL)
    for m in arm_re.finditer(body):
        cat = m.group("cat")
        for v in re.findall(r"Self::(\w+)", m.group("variants")):
            mapping[v] = cat
    return mapping

def main():
    if not TYPES_RS.exists():
        print(f"types.rs not found at {TYPES_RS}", file=sys.stderr)
        return 1
    source = TYPES_RS.read_text(encoding="utf-8")
    
    variants = parse_enum(source, "ContractError")
    categories = extract_category_mapping(source)
    
    print("| Code | Variant | Category | Trigger |")
    print("|:---:|---|---|---|")
    
    # We need to output 1..63
    max_code = 63
    variant_dict = {v[0]: (v[1], v[2]) for v in variants}
    
    for code in range(1, max_code + 1):
        if code in variant_dict:
            name, desc = variant_dict[code]
            cat = categories.get(name, "-")
            print(f"| {code} | `{name}` | {cat} | {desc} |")
        else:
            print(f"| {code} | *(Unused)* | - | - |")
    
    return 0

if __name__ == "__main__":
    sys.exit(main())
