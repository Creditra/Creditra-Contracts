# Moved: `ContractError` taxonomy

This file is now a redirect stub. The canonical error reference is
**[`docs/errors.md`](./errors.md)**.

The category tables and per-category SDK recovery actions that lived here are
now part of the canonical page, alongside the single code table they were
duplicating.

To regenerate or verify the tables from the enum, run:

```bash
python3 scripts/list_contract_errors.py --categories
python3 scripts/list_contract_errors.py --check
```

The source of truth is
[`ContractError`](../contracts/credit/src/types.rs). Do not re-add a taxonomy
here.
