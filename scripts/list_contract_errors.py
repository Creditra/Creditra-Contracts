#!/usr/bin/env python3
"""Print every ContractError variant declared in contracts/credit/src/types.rs.

The script is intentionally dependency-free; it parses the file with a simple
regex rather than running rustc. It is meant as a quick reference for
indexer/SDK authors who need to keep an error-code table in sync.

Usage:
    scripts/list_contract_errors.py                # plain text table
    scripts/list_contract_errors.py --json         # machine-readable JSON
    scripts/list_contract_errors.py --categories   # grouped by category
    scripts/list_contract_errors.py --json --categories  # JSON with category
"""

from __future__ import annotations

import json
import pathlib
import re
import sys

REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent
TYPES_RS = REPO_ROOT / "contracts" / "credit" / "src" / "types.rs"

# Match lines like `    Unauthorized = 1,` inside `pub enum ContractError`.
VARIANT_RE = re.compile(r"^\s*(?P<name>[A-Za-z][A-Za-z0-9]*)\s*=\s*(?P<code>\d+)\s*,")


def parse_variants(source: str, enum_name: str) -> list[tuple[int, str]]:
    enum_open = re.search(rf"pub\s+enum\s+{enum_name}\s*\{{", source)
    if not enum_open:
        raise SystemExit(f"{enum_name} enum not found in types.rs")

    body_start = enum_open.end()
    # Match braces to find the enum body.
    depth = 1
    i = body_start
    while i < len(source) and depth > 0:
        ch = source[i]
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
        i += 1

    body = source[body_start : i - 1]
    variants: list[tuple[int, str]] = []
    for line in body.splitlines():
        m = VARIANT_RE.match(line)
        if m:
            variants.append((int(m.group("code")), m.group("name")))
    variants.sort()
    return variants


def extract_category_mapping(source: str) -> dict[str, list[tuple[int, str]]]:
    """Extract the category→variants mapping from the `category()` method body."""
    # Find the start of category()
    fn_start = re.search(
        r"pub fn category\s*\(&self\)\s*->\s*ContractErrorCategory\s*\{",
        source,
    )
    if not fn_start:
        raise SystemExit("category() method not found in types.rs")

    # Extract the full method body by counting brace depth
    i = fn_start.end()
    depth = 1
    while i < len(source) and depth > 0:
        if source[i] == "{":
            depth += 1
        elif source[i] == "}":
            depth -= 1
        i += 1
    # body is everything inside the outer braces
    body = source[fn_start.end() : i - 1]

    categories: dict[str, list[tuple[int, str]]] = {}
    error_variants = {name: code for code, name in parse_variants(source, "ContractError")}

    # Match multi-line arms: Self::V (| Self::V)* => {? ContractErrorCategory::Cat
    arm_re = re.compile(
        r"(?P<variants>Self::\w+(?:\s*\|\s*Self::\w+)*)\s*=>\s*"
        r"\{?\s*ContractErrorCategory::(?P<cat>\w+)",
        re.DOTALL,
    )
    for m in arm_re.finditer(body):
        cat = m.group("cat")
        if cat not in categories:
            categories[cat] = []
        for v in re.findall(r"Self::(\w+)", m.group("variants")):
            code = error_variants.get(v)
            if code is not None:
                categories[cat].append((code, v))

    for cat in categories:
        categories[cat].sort()
    return categories


def main(argv: list[str]) -> int:
    if not TYPES_RS.exists():
        print(f"types.rs not found at {TYPES_RS}", file=sys.stderr)
        return 1
    source = TYPES_RS.read_text(encoding="utf-8")

    show_categories = "--categories" in argv
    show_json = "--json" in argv

    if show_categories:
        categories = extract_category_mapping(source)
        category_codes = {name: code for code, name in parse_variants(source, "ContractErrorCategory")}

        if show_json:
            output = []
            for cat_name in sorted(categories, key=lambda c: category_codes.get(c, 0)):
                output.append({
                    "category_code": category_codes.get(cat_name),
                    "category_name": cat_name,
                    "variants": [{"code": c, "name": n} for c, n in categories[cat_name]],
                })
            json.dump(output, sys.stdout, indent=2)
            sys.stdout.write("\n")
            return 0

        print(f"{'Cat Code':>8}  {'Category':<14}  {'Code':>4}  Variant")
        print(f"{'--------':>8}  {'--------':<14}  {'----':>4}  -------")
        total = 0
        for cat_name in sorted(categories, key=lambda c: category_codes.get(c, 0)):
            cat_code = category_codes.get(cat_name, 0)
            variants = categories[cat_name]
            for i, (code, name) in enumerate(variants):
                cat_label = cat_name if i == 0 else ""
                cat_code_str = str(cat_code) if i == 0 else ""
                print(f"{cat_code_str:>8}  {cat_label:<14}  {code:>4}  {name}")
                total += 1
            if variants:
                print()
        print(f"{total} variants across {len(categories)} categories")
        return 0

    variants = parse_variants(source, "ContractError")

    if show_json:
        json.dump(
            [{"code": code, "name": name} for code, name in variants],
            sys.stdout,
            indent=2,
        )
        sys.stdout.write("\n")
        return 0

    print(f"{'Code':>4}  Variant")
    print("----  -------")
    for code, name in variants:
        print(f"{code:>4}  {name}")
    print(f"\n{len(variants)} variants")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
