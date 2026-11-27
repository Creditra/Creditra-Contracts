# Credit Score Pre-Commitment (the "VRF" hook)

This document describes the score commitment mechanism in the Creditra credit
contract, and — importantly — **what it does and does not guarantee**.

> **Terminology.** The code, entry points, and storage key keep the `Vrf` prefix
> for ABI and storage compatibility, but the mechanism is **not** a
> verifiable-random-function protocol. Read it as an *admin score
> pre-commitment*. On-chain VRF verification is a tracked follow-up
> ([#1447](https://github.com/Creditra/Creditra-Contracts/issues/1447)).

## What it actually is

1. The admin calls `commit_vrf_output(borrower, commitment_hash)` to store an
   opaque 32-byte value for a borrower.
2. While a commitment exists, `update_risk_parameters` will only accept a new
   `risk_score` equal to the deterministic reduction of that stored value:
   `score = sum(commitment_hash bytes) % 101`.
3. `clear_vrf_commitment(borrower)` lets the admin remove the commitment and
   start over.

That is the entire on-chain behaviour. It gives a **pre-commitment /
change-detection** property: after the hash is published, a score that does not
match it is rejected, so an observer can detect a score that was changed after
the fact.

## What it is *not*

- **No VRF proof is verified.** The contract does not see, and cannot check, a
  VRF proof or public key. It stores bytes the admin supplied.
- **No randomness is generated or verified by the protocol.** The stored hash is
  arbitrary bytes from the admin's perspective; the contract cannot tell a real
  VRF output from a value the admin made up.
- **Not unpredictable and not binding.** `derive_score_from_hash` is public and
  deterministic, so for *any* target score in `[0, 100]` the admin can construct
  a hash that reduces to it — for example, the 32-byte value `[s, 0, …, 0]`
  reduces to `s`. Committing therefore does not stop the admin from arriving at a
  chosen score; it only fixes *which* hash the score must match.
- **Not "cryptographically bound".** The reduction is a lossy arithmetic function,
  not a hash. Many inputs collapse to the same score, and inverting the *goal*
  (find a hash for a desired score) is trivial. Do not describe the mechanism as
  cryptographically binding or non-invertible.
- **Optional.** When no commitment exists, `update_risk_parameters` skips the
  score check entirely (backward compatibility). The mechanism cannot constrain
  an admin who never commits; making it mandatory is tracked by
  [#1240](https://github.com/Creditra/Creditra-Contracts/issues/1240).

## Trust assumptions

The (weaker) change-detection guarantee holds only if **all** of the following
are true. None of them is enforced on-chain:

1. **Genuine VRF origin.** The committed hash really is the output of a VRF whose
   seed and proof the admin could not control or predict at selection time.
2. **Commit before knowledge.** The admin commits before learning the
   information the score is meant to be independent of, and does not grind
   candidate hashes off-chain.
3. **Clear is exceptional.** `clear_vrf_commitment` is used only for genuine
   recovery (for example, an off-chain process failure), not routinely to reset
   the commitment and re-choose a score.
4. **Advisory presentation.** Integrators surface the commitment to users as
   advisory evidence of a pre-committed score, never as cryptographic proof of
   fairness or randomness.

If any of (1)–(3) does not hold, the commitment constrains nothing beyond what
the admin chooses to do. Users should therefore treat the admin as able to
influence scores, with the commitment providing accountability rather than
prevention.

## Score derivation and its distribution

The reduction is:

```text
score = (commitment_hash[0] + commitment_hash[1] + ... + commitment_hash[31]) % 101
```

### Distribution caveat — the output is *not* uniform

The documentation previously claimed the score is "uniformly distributed". That
is inaccurate, for two independent reasons:

- **Modulo bias.** The byte sum lies in `[0, 8160]`, and `8161 = 101 * 80 + 81`.
  So 81 of the 101 residues can be produced by 81 distinct sums while the
  remaining 20 can be produced by 80 — a small but real bias (~1.25%) even if the
  sum were uniformly distributed.
- **The sum itself is not uniform.** A sum of 32 independent bytes is
  bell-shaped, not flat, so the residue distribution is only approximately
  uniform and the tails are slightly under-represented.

Do not model `derive_score_from_hash` as a uniform draw. Measuring and
documenting the actual distribution is tracked by
[#1329](https://github.com/Creditra/Creditra-Contracts/issues/1329).

## Workflow

### 1. Commit

```rust
commit_vrf_output(env, borrower, commitment_hash)
```

- `commitment_hash`: a 32-byte value the revealed score must reduce to.
- Write-once per borrower until cleared; a second commit reverts.

### 2. Reveal

```rust
update_risk_parameters(env, borrower, credit_limit, interest_rate_bps, risk_score)
```

If the score is changing and a commitment exists, the contract checks
`derive_score_from_hash(commitment_hash) == risk_score`; otherwise the update is
rejected with `Unauthorized`. Rate-only updates (score unchanged) skip the check,
and updates with no commitment are allowed.

### 3. Clear (recovery only)

```rust
clear_vrf_commitment(env, borrower)
```

Removes the commitment so a new one can be set. Admin-only.

## API reference

### `commit_vrf_output`

```rust
pub fn commit_vrf_output(env: Env, borrower: Address, commitment_hash: BytesN<32>)
```

- **Authorization**: admin only.
- **Errors**: `Paused`; auth failure; `InvalidAmount` when a commitment already
  exists for the borrower.

### `clear_vrf_commitment`

```rust
pub fn clear_vrf_commitment(env: Env, borrower: Address)
```

- **Authorization**: admin only.
- **Errors**: `Paused`; auth failure.

### `get_vrf_commitment`

```rust
pub fn get_vrf_commitment(env: Env, borrower: Address) -> Option<VrfCommitment>
```

- **Authorization**: public.
- **Returns**: the commitment (hash plus `committed_at`), or `None`.

### `verify_vrf_commitment`

```rust
pub fn verify_vrf_commitment(env: &Env, borrower: &Address, risk_score: u32) -> bool
```

- **Authorization**: internal (called by `update_risk_parameters`).
- **Returns**: `true` when `risk_score` equals the reduction of the stored hash.
  This is an equality check — it verifies **no** proof and does **not** establish
  that the hash came from a VRF.
- **Errors**: `CreditLineNotFound` when no commitment exists.

## Integration with risk parameters

```rust
// The score check applies only when the score actually changes and a
// commitment exists. It compares against sum(bytes) % 101 of the stored hash;
// it does not authenticate the hash as a VRF output.
if risk_score != credit_line.risk_score {
    if let Some(_commitment) = crate::scoring::get_vrf_commitment(&env, &borrower) {
        if !crate::scoring::verify_vrf_commitment(&env, &borrower, risk_score) {
            env.panic_with_error(ContractError::Unauthorized);
        }
    }
    // No commitment → no check (backward compatibility).
}
```

## Storage layout

- **Tier**: persistent
- **Key**: `DataKey::VrfCommitment(Address)`
- **Value**: `VrfCommitment { commitment_hash: BytesN<32>, committed_at: u64 }`
- **TTL**: refreshed on access alongside the borrower's credit-line entry

## Testing

The existing tests cover deterministic derivation, range, and the commit /
verify / clear workflow (`contracts/credit/tests/vrf_commitment.rs`). They assert
*determinism*, not unpredictability or uniformity. A seeded distribution
measurement is tracked separately by
[#1329](https://github.com/Creditra/Creditra-Contracts/issues/1329).

## Follow-ups (not in this change)

| Topic | Issue |
| --- | --- |
| Verify real VRF proofs / authenticated oracle output on-chain | [#1447](https://github.com/Creditra/Creditra-Contracts/issues/1447) |
| Measure and document the score distribution | [#1329](https://github.com/Creditra/Creditra-Contracts/issues/1329) |
| Optionally mandate commitments for score changes | [#1240](https://github.com/Creditra/Creditra-Contracts/issues/1240) |
| Consume the commitment after a verified score change | [#1239](https://github.com/Creditra/Creditra-Contracts/issues/1239) |

## References

- Implementation: `contracts/credit/src/scoring.rs`
- Integration: `contracts/credit/src/risk.rs` (`update_risk_parameters`)
- Storage: `contracts/credit/src/storage.rs` (`DataKey::VrfCommitment`)
- Related: `docs/RISK_PRICING.md`
