#!/usr/bin/env python3
"""Print every ContractError variant declared in contracts/credit/src/types.rs.

The script is intentionally dependency-free; it parses the file with a simple
regex rather than running rustc. It is meant as a quick reference for
indexer/SDK authors who need to keep an error-code table in sync.

`docs/errors.md` is the canonical published table, so `--check` verifies it
against the enum instead of printing.

Usage:
    scripts/list_contract_errors.py                # plain text table
    scripts/list_contract_errors.py --json         # machine-readable JSON
    scripts/list_contract_errors.py --categories   # grouped by category
    scripts/list_contract_errors.py --json --categories  # JSON with category
    scripts/list_contract_errors.py --check        # verify docs/errors.md
"""

from __future__ import annotations

import json
import pathlib
import re
import sys

REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent
TYPES_RS = REPO_ROOT / "contracts" / "credit" / "src" / "types.rs"
CANONICAL_DOC = REPO_ROOT / "docs" / "errors.md"

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
    # `category()` globs the enum into scope, so the arm target is a bare name.
    arm_re = re.compile(
        r"(?P<variants>Self::\w+(?:\s*\|\s*Self::\w+)*)\s*=>\s*"
        r"\{?\s*(?:ContractErrorCategory::)?(?P<cat>\w+)"
    )
    known = {name for code, name in parse_variants(source, "ContractErrorCategory")}
    for m in arm_re.finditer(body):
        cat = m.group("cat")
        if cat not in known:
            continue
        if cat not in categories:
            categories[cat] = []
        for v in re.findall(r"Self::(\w+)", m.group("variants")):
            code = error_variants.get(v)
            if code is not None:
                categories[cat].append((code, v))

    for cat in categories:
        categories[cat].sort()
    return categories


def doc_section(doc: str, heading: str) -> str:
    """Body of a `## <heading>` section, up to the next `---` rule."""
    if f"## {heading}" not in doc:
        raise SystemExit(f"{CANONICAL_DOC.name} is missing a '## {heading}' section")
    body = doc.split(f"## {heading}", 1)[1]
    if "\n---" not in body:
        raise SystemExit(f"the '{heading}' section in {CANONICAL_DOC.name} is not closed by `---`")
    return body.split("\n---", 1)[0]


def table_cells(line: str) -> list[str]:
    return [cell for cell in (c.strip() for c in line.split("|")) if cell]


def check_canonical_doc(source: str) -> int:
    """Fail if the published canonical table drifts from the enum."""
    if not CANONICAL_DOC.exists():
        print(f"canonical error reference not found at {CANONICAL_DOC}", file=sys.stderr)
        return 1

    doc = CANONICAL_DOC.read_text(encoding="utf-8")
    variants = parse_variants(source, "ContractError")
    expected = {code: (name, None) for code, name in variants}

    published: dict[int, tuple[str, str]] = {}
    for line in doc_section(doc, "Error Code Table").splitlines():
        cells = table_cells(line)
        if len(cells) < 3 or not cells[0].strip("`").isdigit():
            continue  # header / separator row
        code = int(cells[0].strip("`"))
        if code in published:
            print(f"{CANONICAL_DOC.name}: code {code} is published twice", file=sys.stderr)
            return 1
        published[code] = (cells[1].strip("`"), cells[2].strip("`"))

    problems: list[str] = []
    for code, name in variants:
        if code not in published:
            problems.append(f"code {code} ({name}) is missing from {CANONICAL_DOC.name}")
        elif published[code][0] != name:
            problems.append(
                f"code {code} is `{published[code][0]}`, expected `{name}`"
            )
    for code, (name, _) in published.items():
        if code not in expected:
            problems.append(f"{CANONICAL_DOC.name} publishes code {code} (`{name}`), which the contract cannot emit")

    if problems:
        print(f"{CANONICAL_DOC.name} has drifted from ContractError:", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        return 1

    print(f"{CANONICAL_DOC.name} matches ContractError ({len(variants)} variants)")
    return 0


def main(argv: list[str]) -> int:
    if not TYPES_RS.exists():
        print(f"types.rs not found at {TYPES_RS}", file=sys.stderr)
        return 1
    source = TYPES_RS.read_text(encoding="utf-8")

    show_categories = "--categories" in argv
    show_json = "--json" in argv

    if "--check" in argv:
        return check_canonical_doc(source)

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
