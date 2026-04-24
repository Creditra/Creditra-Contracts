# test(credit): borrower key encoding and storage round-trip tests

## Summary

Adds 12 deterministic tests confirming that borrower `Address` values are stored and retrieved consistently as persistent storage keys, with no collisions, no cross-contamination between slots, and correct scoping of all mutations.

---

## What Changed

New test module `test_borrower_key_encoding` in `contracts/credit/src/lib.rs`.

### Storage key encoding in Soroban

Soroban persistent storage uses the XDR encoding of the key value. When an `Address` is used directly as a key (as this contract does: `env.storage().persistent().get(&borrower)`), each distinct address maps to a unique, deterministic storage slot. These tests verify that property holds in practice.

### Tests added

| Test | What it verifies |
|---|---|
| `distinct_addresses_have_independent_slots` | Writing for A does not create an entry for B |
| `storage_round_trip_preserves_all_fields` | All fields written by `open_credit_line` are read back unchanged |
| `multiple_borrowers_stored_independently` | 5 borrowers with distinct limits each read back their own data |
| `mutation_of_one_slot_does_not_affect_another` | Suspending A does not change B's status |
| `default_scoped_to_correct_borrower` | Defaulting A does not affect B |
| `close_scoped_to_correct_borrower` | Closing A does not affect B |
| `update_risk_parameters_scoped_to_correct_borrower` | Risk param update on A leaves B unchanged |
| `reopen_after_close_overwrites_slot_with_fresh_data` | Re-opening after close uses same key, fresh data |
| `get_credit_line_returns_none_for_unknown_address` | Unknown address returns `None` |
| `large_batch_no_cross_contamination` | 10 borrowers with sequential limits all read back correctly |
| `stored_borrower_field_matches_key` | `CreditLineData.borrower` field equals the address used as key |
| `duplicate_open_for_active_borrower_reverts` | Second open for same active borrower panics |

---

## Pre-existing fixes applied

Same set as previous PRs on this branch:

- Inline `config::`, `query::`, `risk::` undeclared module calls
- Add `ContractError` to `use types::{}` import
- Add missing `accrued_interest`, `last_accrual_ts` fields to `CreditLineData` init
- Add SPDX header to `lib.rs`
- Add `setup`/`approve` helpers to `mod test`; remove dead helpers
- Fix broken test bodies (`test_suspend_nonexistent`, `suspend_defaulted_line_reverts`, `test_multiple_borrowers`, `test_draw_credit_updates_utilized`)
- Update `reinstate_credit_line` call sites to pass `target_status`
- Add `#[allow(dead_code)]` to events.rs v2 publish functions
- Fix non-exhaustive `CreditStatus` match in `duplicate_open_policy.rs`
- Fix unused `token_id` variable

---

## Test Results

```
test result: ok. 78 passed; 0 failed  (lib — includes 12 new key encoding tests)
test result: ok. 28 passed; 0 failed  (integration)
test result: ok. 3 passed;  0 failed  (spdx_header_bug_exploration)
test result: ok. 6 passed;  0 failed  (spdx_preservation)
test result: ok. 7 passed;  0 failed  (duplicate_open_policy)
```

---

Closes issue #267
