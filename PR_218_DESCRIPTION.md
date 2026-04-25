# feat(credit): Admin-only liquidity source configuration with tests

## Summary

Implements `set_liquidity_source` as a fully documented, admin-only contract function that controls where draw tokens come from and where repayment tokens go. Adds `get_liquidity_source` as a companion view function. Includes 10 targeted tests covering all paths.

---

## What Changed

### `set_liquidity_source` (contracts/credit/src/lib.rs)

Previously delegated to an undeclared `config::` module (compilation error). Now fully inlined with:

- `require_admin_auth` guard
- Persists to `DataKey::LiquiditySource` in instance storage
- Full `///` doc comments covering parameters, trust model, and failure modes

### `get_liquidity_source` (new view function)

Returns the current reserve address, defaulting to the contract's own address if never set.

### Reserve semantics

| Operation | Behaviour |
|---|---|
| `draw_credit` | Transfers tokens **from** liquidity source **to** borrower |
| `repay_credit` | Transfers tokens **from** borrower **to** liquidity source |

After `init`, the liquidity source defaults to the contract address. Call `set_liquidity_source` to redirect to an external vault.

### Other fixes applied

- Inline `config::`, `query::`, `risk::` undeclared module calls
- Add `ContractError` to imports
- Add missing `accrued_interest` / `last_accrual_ts` fields to `CreditLineData` init
- Add SPDX header to `lib.rs`
- Fix pre-existing broken test bodies and dead helpers
- Add `#[allow(dead_code)]` to events.rs v2 publish functions
- Fix non-exhaustive `CreditStatus` match in `duplicate_open_policy.rs`

---

## Tests (`mod test_liquidity_source`)

10 tests covering every path:

| Test | What it covers |
|---|---|
| `default_liquidity_source_is_contract_address` | After `init`, source == contract address |
| `admin_can_set_liquidity_source` | Admin sets external reserve; view returns it |
| `liquidity_source_persists_and_can_be_updated` | Overwriting persists correctly |
| `non_admin_cannot_set_liquidity_source` | Non-admin call panics |
| `draw_credit_pulls_from_configured_reserve` | Draw deducts from external reserve |
| `repay_credit_sends_to_configured_reserve` | Repayment goes to external reserve |
| `draw_uses_contract_as_default_reserve` | Without explicit set, draw uses contract balance |
| `repay_uses_contract_as_default_reserve` | Without explicit set, repayment goes to contract |
| `switching_reserve_mid_lifecycle_routes_correctly` | Changing reserve mid-lifecycle routes next draw correctly |
| `get_liquidity_source_reflects_latest_value` | View function tracks every update |

---

## Test Results

```
test result: ok. 76 passed; 0 failed  (lib)
test result: ok. 28 passed; 0 failed  (integration)
test result: ok. 3 passed;  0 failed  (spdx)
test result: ok. 6 passed;  0 failed  (spdx_preservation)
test result: ok. 7 passed;  0 failed  (duplicate_open_policy)
```

---

## Security Notes

- Only the admin can call `set_liquidity_source`. Admin key should be a multisig in production.
- A compromised admin could redirect repayments to an arbitrary address. This is the same trust boundary as all other admin-only functions.
- When using an external reserve, that vault must hold sufficient token balance for draws to succeed.
- Failure mode: draw with insufficient reserve balance panics with "Insufficient liquidity reserve for requested draw amount".

---

Closes issue #218
