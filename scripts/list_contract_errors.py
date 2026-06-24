#!/usr/bin/env python3
"""Print every ContractError variant declared in contracts/credit/src/types.rs.

The script is intentionally dependency-free; it parses the file with a simple
regex rather than running rustc. It is meant as a quick reference for
indexer/SDK authors who need to keep an error-code table in sync.

Usage:
    scripts/list_contract_errors.py            # plain text table
    scripts/list_contract_errors.py --json     # machine-readable JSON
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


def parse_variants(source: str) -> list[tuple[int, str]]:
    enum_open = re.search(r"pub\s+enum\s+ContractError\s*\{", source)
    if not enum_open:
        raise SystemExit("ContractError enum not found in types.rs")

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


def main(argv: list[str]) -> int:
    if not TYPES_RS.exists():
        print(f"types.rs not found at {TYPES_RS}", file=sys.stderr)
        return 1
    source = TYPES_RS.read_text(encoding="utf-8")
    variants = parse_variants(source)

    if "--json" in argv:
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
