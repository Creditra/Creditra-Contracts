# Credit Score Commitment and Oracle Proof Verification

This document describes how the Creditra credit contract authenticates the
32-byte value that seeds a borrower's risk score, and what the mechanism does
and does not guarantee.

> **Terminology.** Entry points and storage keys keep the `Vrf` prefix for ABI
> and storage compatibility. The contract does **not** run a VRF. It
> authenticates the output of an oracle that is trusted to run one, using an
> Ed25519 signature checked with the Soroban host's verifier
> ([#1447](https://github.com/Creditra/Creditra-Contracts/issues/1447)).

## Two commit paths

### Verified (recommended)

1. The admin configures the oracle key with
   `set_vrf_oracle_public_key(oracle_public_key)`.
2. Anyone submits `commit_vrf_output_verified(borrower, output, proof)`.
   The contract rebuilds the signed message, verifies the 64-byte Ed25519 proof
   against the configured key, and only then stores the commitment.
3. While a verified commitment exists, `update_risk_parameters` accepts only a
   new `risk_score` equal to `sum(output bytes) % 101`, and only while the
   commitment is still backed by a proof under the **current** key.

Verified commits are permissionless: the proof is the authorization, so the
admin does not gatekeep the verified path.

### Legacy (unverified)

`commit_vrf_output(borrower, value)` stores a 32-byte value supplied by the
admin with no proof. It is kept for ABI compatibility, but it reverts with
`VrfProofRequired` once an oracle key is configured. Score changes that rely on
a legacy commitment are also rejected with `VrfProofRequired` after the key is
configured.

## Proof message

The signed message is the concatenation of Soroban XDR values:

```text
b"CreditraVRF/v1"
|| ScVal::Address(contract_id).to_xdr()
|| ScVal::Address(borrower).to_xdr()
|| ScVal::Bytes(output).to_xdr()
```

`vrf_proof_message(env, borrower, output)` exposes the encoding so off-chain
signers can reproduce it byte-for-byte.

- Binding the **contract address** prevents a proof for one deployment from
  being replayed on another.
- Binding the **borrower** prevents cross-borrower replay.
- Binding the **output** ties the signature to the exact 32 bytes that seed the
  score.

The message carries **no epoch or freshness deadline**. The oracle must not sign
competing outputs for the same borrower in the same cycle; if it does, whichever
output is submitted first wins.

## What the verified path guarantees

- The stored commitment value is the exact output covered by a valid oracle
  signature over this contract and this borrower.
- Once an oracle key is configured, the admin cannot substitute a commitment
  value of their own through the legacy path.
- A score change is accepted only if it equals the reduction of that verified
  output.

## What it does **not** guarantee

- **No VRF is executed on-chain.** The contract authenticates an oracle's
  output; it cannot tell whether the oracle ran a VRF correctly.
- **Not unpredictable by itself.** The security depends on the oracle key being
  controlled independently of the admin. An admin who can rotate the key to one
  they control can forge outputs.
- **Not binding in a cryptographic-hash sense.** `derive_score_from_hash` is a
  public, deterministic, lossy reduction, not a hash. Many outputs collapse to
  the same score.
- **Optional.** When no commitment exists, `update_risk_parameters` skips the
  score check entirely (backward compatibility). Making commitments mandatory is
  tracked by [#1240](https://github.com/Creditra/Creditra-Contracts/issues/1240).
- **No freshness.** An old oracle output remains valid until it is cleared or the
  key is rotated.

## Trust assumptions

The guarantee holds only while all of the following hold:

1. **The oracle key is independent.** The key set with
   `set_vrf_oracle_public_key` belongs to an oracle whose signing policy is not
   controlled by the same party that sets scores. Production deployments should
   place the admin role behind a governance contract or multisig with a timelock.
2. **The oracle signs once per cycle.** The message binds the borrower and output
   but carries no epoch, so the oracle must not sign competing outputs for the
   same borrower.
3. **The oracle is live.** If the oracle stops signing, verified score changes
   are impossible. Clearing a commitment still allows an unconstrained score
   change, and the legacy path is only blocked while a key is configured.
4. **Key rotation is deliberate.** `is_vrf_commitment_verified` requires the
   recorded proof to use the *currently* configured key. Rotating the key
   invalidates existing verified commitments until they are cleared and
   re-committed. This lets governance revoke a compromised key.
5. **Clear is exceptional.** `clear_vrf_commitment` is admin-only and removes
   both the commitment and its attestation. Routine use defeats the
   change-detection property.

## Score derivation and its distribution

The reduction is:

```text
score = (output[0] + output[1] + ... + output[31]) % 101
```

### Distribution caveat — the output is *not* uniform

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

### 1. Configure the oracle key (admin)

```rust
set_vrf_oracle_public_key(env, oracle_public_key)
```

- `oracle_public_key`: 32-byte Ed25519 public key. An all-zero key is rejected.
- The key cannot be removed. Setting it again rotates the key and invalidates
  existing verified commitments.

### 2. Commit a verified output (permissionless)

```rust
commit_vrf_output_verified(env, borrower, output, proof)
```

- `output`: 32-byte oracle output the score must reduce to.
- `proof`: 64-byte Ed25519 signature over the proof message above.
- Write-once per borrower until cleared; a second commit reverts with
  `InvalidAmount`.
- An invalid proof reverts with the host error
  `ScErrorType::Crypto` / `ScErrorCode::InvalidInput`
  (`"failed ED25519 verification"`). The contract cannot catch host crypto
  errors, so this is the dedicated failure for a bad signature.

### 3. Reveal a score

```rust
update_risk_parameters(env, borrower, credit_limit, interest_rate_bps, risk_score)
```

If the score is changing and a commitment exists:

- When an oracle key is configured, the commitment must be verified under the
  current key; otherwise the update reverts with `VrfProofRequired`.
- The score must equal `derive_score_from_hash(output)`; otherwise the update
  reverts with `Unauthorized`.

Rate-only updates (score unchanged) skip the check. Updates with no commitment
are allowed.

### 4. Clear (recovery only)

```rust
clear_vrf_commitment(env, borrower)
```

Removes the commitment and its attestation so a new one can be set. Admin-only.

### Legacy commit (before a key is configured)

```rust
commit_vrf_output(env, borrower, commitment_hash)
```

Stores an unauthenticated 32-byte value. Reverts with `VrfProofRequired` once an
oracle key is configured.

## API reference

### `set_vrf_oracle_public_key`

```rust
pub fn set_vrf_oracle_public_key(env: Env, public_key: BytesN<32>)
```

- **Authorization**: admin only.
- **Errors**: `Paused`; auth failure; `InvalidAmount` for an all-zero key.

### `get_vrf_oracle_public_key`

```rust
pub fn get_vrf_oracle_public_key(env: Env) -> Option<BytesN<32>>
```

- **Authorization**: public.

### `commit_vrf_output_verified`

```rust
pub fn commit_vrf_output_verified(
    env: Env,
    borrower: Address,
    output: BytesN<32>,
    proof: BytesN<64>,
)
```

- **Authorization**: public (the proof authorizes the commit).
- **Errors**: `Paused`; `VrfOracleNotConfigured`; `InvalidAmount` when a
  commitment already exists; host Ed25519 error for an invalid proof.

### `get_vrf_verification`

```rust
pub fn get_vrf_verification(env: Env, borrower: Address) -> Option<VrfVerification>
```

- **Authorization**: public.
- **Returns**: `VrfVerification { output, oracle_public_key, verified_at }`, or
  `None`.

### `is_vrf_commitment_verified`

```rust
pub fn is_vrf_commitment_verified(env: Env, borrower: Address) -> bool
```

- **Authorization**: public.
- **Returns**: `true` only when the stored commitment matches the attested output
  and the attestation uses the currently configured key. Legacy commitments and
  commitments made before a key rotation return `false`.

### `commit_vrf_output` (legacy)

```rust
pub fn commit_vrf_output(env: Env, borrower: Address, commitment_hash: BytesN<32>)
```

- **Authorization**: admin only.
- **Errors**: `Paused`; auth failure; `VrfProofRequired` when a key is
  configured; `InvalidAmount` when a commitment already exists.

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
- **Returns**: `VrfCommitment { commitment_hash, committed_at }`, or `None`.

### `verify_vrf_commitment`

```rust
pub fn verify_vrf_commitment(env: &Env, borrower: &Address, risk_score: u32) -> bool
```

- **Authorization**: internal (called by `update_risk_parameters`).
- **Returns**: `true` when `risk_score` equals the reduction of the stored value.
  This is an equality check and does not inspect a proof; pair it with
  `is_vrf_commitment_verified` when attestation matters.
- **Errors**: `MissingVrfCommitment` when no commitment exists.

## Integration with risk parameters

```rust
if risk_score != credit_line.risk_score {
    if let Some(_commitment) = crate::scoring::get_vrf_commitment(&env, &borrower) {
        if crate::scoring::get_vrf_oracle_public_key(&env).is_some()
            && !crate::scoring::is_vrf_commitment_verified(&env, &borrower)
        {
            env.panic_with_error(ContractError::VrfProofRequired);
        }
        if !crate::scoring::verify_vrf_commitment(&env, &borrower, risk_score) {
            env.panic_with_error(ContractError::Unauthorized);
        }
    }
    // No commitment → no check (backward compatibility).
}
```

## Storage layout

- **`DataKey::VrfCommitment(Address)`** — persistent
  - Value: `VrfCommitment { commitment_hash: BytesN<32>, committed_at: u64 }`
  - TTL: refreshed on access alongside the borrower's credit-line entry.
- **`DataKey::VrfVerification(Address)`** — persistent
  - Value: `VrfVerification { output: BytesN<32>, oracle_public_key: BytesN<32>, verified_at: u64 }`
  - TTL: refreshed on read and on commit.
- **`DataKey::VrfOraclePublicKey`** — instance
  - Value: `BytesN<32>` (Ed25519 public key).

## Testing

`contracts/credit/tests/vrf_commitment.rs` covers:

- deterministic derivation and the fixed-seed distribution check;
- the legacy commit / verify / clear workflow;
- verified commits with a real Ed25519 signature;
- rejection of wrong signatures, cross-borrower replay, and cross-contract
  replay;
- `VrfOracleNotConfigured`, `VrfProofRequired`, duplicate-commit, pause, and
  zero-key errors;
- score acceptance/rejection through `update_risk_parameters`;
- key rotation invalidating existing attestations;
- TTL refresh for the attestation record.

## Follow-ups (not in this change)

| Topic | Issue |
| --- | --- |
| Measure and document the score distribution | [#1329](https://github.com/Creditra/Creditra-Contracts/issues/1329) |
| Optionally mandate commitments for score changes | [#1240](https://github.com/Creditra/Creditra-Contracts/issues/1240) |
| Consume the commitment after a verified score change | [#1239](https://github.com/Creditra/Creditra-Contracts/issues/1239) |

## References

- Implementation: `contracts/credit/src/scoring.rs`
- Integration: `contracts/credit/src/risk.rs` (`update_risk_parameters`)
- Storage: `contracts/credit/src/storage.rs` (`DataKey::VrfCommitment`,
  `DataKey::VrfVerification`, `DataKey::VrfOraclePublicKey`)
- Related: `docs/RISK_PRICING.md`
