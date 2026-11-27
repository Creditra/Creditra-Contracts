# `creditra-risk` — standalone risk-admin cooldown contract

A small, **self-contained** Soroban contract that enforces a time-based circuit
breaker between admin actions on risk-critical parameters, and keeps its own
instance storage alive by bumping the instance TTL on every entrypoint.

It is intentionally **not** a risk engine and holds **no** credit, pricing, or
borrower state. Its single piece of mutable configuration is a cooldown
duration.

| | |
|---|---|
| Crate | `creditra-risk` (`contracts/risk`) |
| Contract type | `RiskContract` (`src/lib.rs`) |
| Modules | [`src/lib.rs`](./src/lib.rs) (entrypoints), [`src/admin.rs`](./src/admin.rs) (storage + guard), [`src/events.rs`](./src/events.rs) (event payloads) |
| Fuzz target | [`fuzz/`](./fuzz/README.md) — `cargo fuzz`, invariants + auth + overflow |
| Published README | this file (declared via `readme = "README.md"` in `Cargo.toml`) |

---

## 1. Why it exists

A compromised admin key can otherwise change risk parameters as fast as blocks
are produced. This contract gives an operator a single, cheap lever — a
cooldown window — that caps how often a *recorded* risk-admin action can be
registered, so other admins and monitoring systems get a detection window.

The mechanism is deliberately dumb: one `u64` duration, one `u64` timestamp,
one guard. Read `assert_risk_admin_cooldown_elapsed` and you have read the whole
policy.

**A cooldown of `0` (the default) disables enforcement entirely.** Nothing is
blocked until an admin opts in.

---

## 2. Relationship to the credit contract's own cooldown

The credit contract (`contracts/credit`) implements a **parallel, independent**
cooldown with the same semantics, the same storage-key symbols, the same
constant values, and the same error discriminant. Understanding the difference
matters, because **they do not share state and neither one controls the
other**.

| | `creditra-risk` (this crate) | `creditra-credit` (`contracts/credit`) |
|---|---|---|
| Relevant entrypoints | `set_risk_admin_cooldown`, `get_risk_admin_cooldown`, `record_risk_admin_action` | `set_risk_admin_cooldown`, `get_risk_admin_cooldown` (+ the guarded `update_risk_parameters`) |
| Implementation | `src/admin.rs` | `src/storage.rs`, guarded from `src/risk.rs` |
| Storage keys | `Symbol("rad_cool")`, `Symbol("rad_last")` | `Symbol("rad_cool")`, `Symbol("rad_last")` |
| Storage scope | This contract's **own** instance storage | The credit contract's **own** instance storage |
| Who writes `rad_last` | **Only** the explicit `record_risk_admin_action` call | **Automatically**, at the end of every successful `update_risk_parameters` |
| Cooldown guard applies to | `record_risk_admin_action` **only** | `update_risk_parameters` **only** |
| Error on rejection | `RiskAdminCooldownActive` = `54` | `RiskAdminCooldownActive` = `54` |
| TTL constants | `INSTANCE_BUMP_AMOUNT` / `INSTANCE_BUMP_THRESHOLD` | same values, same policy |
| Pause flag | Own `Symbol("paused")`; blocks `set_risk_admin_cooldown` + `record_risk_admin_action` | Credit contract's own global pause flag |
| CI coverage | **none** — no workflow job builds this crate | `contracts/creditra-credit` is the only crate CI builds/tests |

Key consequences:

- **Identical key names, separate namespaces.** `rad_cool` set on the credit
  contract has **no effect** on this contract, and vice versa. There is no
  migration or mirroring path.
- **No cross-contract calls in either direction.** `creditra-risk` does not
  import, invoke, or read `creditra-credit`, and the credit contract does not
  call `record_risk_admin_action`. Each must be configured and called
  separately by the operator.
- **The write is manual here, automatic there.** In the credit contract the
  cooldown timestamp is written by the same transaction that mutates the risk
  parameters, so it cannot drift from the mutation it guards. In this contract
  the operator must submit `record_risk_admin_action` as **its own
  transaction**. If it is never called, `rad_last` stays `0` and the cooldown
  is never enforced (see `assert_risk_admin_cooldown_elapsed`: `last_ts == 0`
  short-circuits to "allow").
- **Duplicated logic, not shared logic.** The two implementations are copies.
  A change to one is not a change to the other.

### Who calls `record_risk_admin_action`?

Exactly one caller kind: **the admin, via an explicit transaction** — a
governance queue, an ops runbook step, or a keeper that co-submits it alongside
whatever off-chain action the cooldown is meant to rate-limit.

Nothing on-chain calls it. It does not mutate any risk parameter, does not
touch the credit contract, and does not read `rad_cool`/`rad_last` from
anywhere but its own instance storage. Its only effects are: (1) revert if the
cooldown is active, (2) write `rad_last = ledger.timestamp`, (3) emit
`("risk","rad_act")`.

> If your intended failure mode is "a stolen admin key cannot rapidly change
> risk parameters", this contract only helps if the *parameter mutation* and
> the *`record_risk_admin_action` call* are deployed together under one
> governance procedure. It is a rate limiter on a bookkeeping call, not on the
> credit contract's parameters. See §8.

---

## 3. Entrypoints

Every entrypoint bumps instance storage TTL when the remaining TTL is below the
threshold (§5), including the read-only views.

| Entrypoint | Auth | Cooldown guard | Errors | Events |
|---|---|---|---|---|
| `init(admin: Address)` | `admin.require_auth()` | — | — | `("risk","init")` |
| `set_risk_admin_cooldown(seconds: u64)` | admin | **not applied** | `Paused` (3), `NotAdmin` (2) | `("risk","rad_cool")` |
| `get_risk_admin_cooldown() -> u64` | none | — | — | — |
| `record_risk_admin_action()` | admin | **applied** | `Paused` (3), `NotAdmin` (2), `RiskAdminCooldownActive` (54) | `("risk","rad_act")` |
| `set_paused(paused: bool)` | admin | **not applied** | `NotAdmin` (2) | `("risk","paused")` / `("risk","unpaused")` |
| `get_admin() -> Address` | none | — | panics `"admin not initialized"` | — |

### `init`

Stores `admin` under `Symbol("admin")` and emits `("risk","init")`.

The rustdoc says "Can only be called once", but **no such check is
implemented** — see §8, gap 1. Treat `init` as a setter for the admin address.

### `set_risk_admin_cooldown`

`seconds = 0` disables enforcement (default, backward compatible). The value is
written to `rad_cool` verbatim; there is no maximum. **The cooldown guard is
not applied to this entrypoint**, so an admin can shorten or disable the
cooldown at any time — including immediately after a `record_risk_admin_action`
that the cooldown would otherwise rate-limit. Combined with `init` (gap 1) and
`set_paused` (below), the circuit breaker therefore constrains exactly one
entrypoint.

### `record_risk_admin_action`

The only guarded entrypoint. Order of operations: `assert_not_paused` →
`require_admin_auth` → `assert_risk_admin_cooldown_elapsed` → write
`rad_last = now` → emit.

Because `assert_risk_admin_cooldown_elapsed` returns early when `rad_last == 0`,
the **first call always succeeds** regardless of the configured cooldown; the
guard only bites from the second call onward.

### `set_paused`

When `paused == true`, `set_risk_admin_cooldown` and
`record_risk_admin_action` revert with `Paused`. `set_paused(false)` and the
read-only views remain available, so the contract can always be unpaused. The
paused flag is read as `false` when the key is absent, so a freshly deployed
(but un-paused) contract behaves as unpaused.

### `get_admin`

Panics with the string `"admin not initialized"` (not a `ContractError`) if
`init` has not run.

---

## 4. Error codes

Discriminants are ABI-stable — do not reorder or renumber.

| Code | Variant | Category (`ContractError::category()`) | Raised when |
|---|---|---|---|
| `1` | `Unauthorized` | `Auth` (`1`) | **Never raised** by any code path (see §8, gap 4) |
| `2` | `NotAdmin` | `Auth` (`1`) | No admin stored, or non-admin caller |
| `3` | `Paused` | `Risk` (`6`) | A blocked entrypoint is called while paused |
| `54` | `RiskAdminCooldownActive` | `Risk` (`6`) | `record_risk_admin_action` before the cooldown elapsed |

`ContractErrorCategory` discriminants: `Auth = 1`, `Risk = 6`.

---

## 5. Storage and TTL policy

All state is in **instance** storage. There is no persistent storage and no
per-borrower state.

| Key | Type | Default when absent | Written by |
|---|---|---|---|
| `Symbol("admin")` | `Address` | none (read panics) | `init` |
| `Symbol("rad_cool")` | `u64` | `0` → cooldown disabled | `set_risk_admin_cooldown` |
| `Symbol("rad_last")` | `u64` | `0` → "no prior action", guard allows | `record_risk_admin_action` |
| `Symbol("paused")` | `bool` | `false` → not paused | `set_paused` |

Keys are `symbol_short!` literals (≤ 9 ASCII characters).

**TTL policy** (identical to `contracts/credit/src/storage.rs`):

| Constant | Ledgers | ≈ Duration |
|---|---|---|
| `INSTANCE_BUMP_AMOUNT` (extend-to) | `3_110_400` | ~6 months |
| `INSTANCE_BUMP_THRESHOLD` (below-which) | `1_555_200` | ~3 months |

`admin::bump_instance_ttl` is a **no-op when the remaining TTL is already above
the threshold** (no ledger write), and every entrypoint calls it — so a
contract that is only being *queried* still never archives. The 2:1 ratio keeps
average TTL writes to at most one per three months for an actively used
contract.

---

## 6. Events

All topics are `(Symbol("risk"), <second topic>)`.

| Topics | Payload | Emitted by |
|---|---|---|
| `("risk","init")` | `RiskInitializedEvent { admin }` | `init` |
| `("risk","rad_cool")` | `RiskAdminCooldownConfiguredEvent { cooldown_seconds }` | `set_risk_admin_cooldown` |
| `("risk","rad_act")` | `RiskAdminActionRecordedEvent { timestamp }` | `record_risk_admin_action` |
| `("risk","paused")` / `("risk","unpaused")` | `RiskPausedEvent { paused }` | `set_paused` |

Note the topic **differs by state**: `set_paused(true)` emits topic
`"paused"`, `set_paused(false)` emits `"unpaused"`. Consumers that filter on a
single topic will miss half the transitions.

`get_admin` and the other read-only views emit nothing.

---

## 7. Building, testing, fuzzing

```bash
# from the repository root; the toolchain is pinned in rust-toolchain.toml
cargo build -p creditra-risk
cargo test  -p creditra-risk
cargo clippy -p creditra-risk --all-targets -- -D warnings
cargo fmt -p creditra-risk -- --check
```

Three integration targets (`risk_admin_cooldown`, `proptest`, `risk_fuzz_tests`)
are gated behind the `instrument` feature:

```bash
cargo test -p creditra-risk --features instrument
```

> **This does not currently compile** (see §8, gap 9): `tests/proptest.rs`
> destructures the legacy `Event` struct shape (`.topics`, `.data`) that
> `soroban-sdk` 22 no longer exposes. The default-feature run above is green
> (**29 tests**); the `instrument` targets are not.

Fuzzing (see [`fuzz/README.md`](./fuzz/README.md)):

```bash
cargo fuzz run --manifest-path contracts/risk/fuzz/Cargo.toml main -- -max_total_time=60
```

Test layout:

| Target | Covers |
|---|---|
| `tests/capabilities.rs` | init + `get_admin`, cooldown default after init |
| `tests/risk_admin_cooldown.rs` | Set/get cooldown, `0` disables, rapid-mutation blocking, elapse boundary, non-admin rejection, first-action-always-succeeds (`instrument`) |
| `tests/rustdoc_risk_tests.rs` | Every documented API contract: discriminants, `category()`, storage keys, event topics/payloads, error paths |
| `tests/ttl_bump.rs` | Every entrypoint bumps TTL; no-op above threshold |
| `tests/events.rs` | Event emission incl. the `paused`/`unpaused` split |
| `tests/proptest.rs`, `tests/risk_fuzz_tests.rs` | Property/invariant coverage — **currently does not compile** (§8, gap 9) |

---

## 8. Known gaps

Real behaviour, reachable today, not currently fixed. Listed so integrators do
not have to rediscover them.

1. **`init` can be called more than once.** It performs `admin.require_auth()`
   and then overwrites `Symbol("admin")` with no `has()` guard, so the current
   admin's key is not privileged: any address can call `init(itself)` and take
   the contract over. The rustdoc ("Can only be called once") does not match the
   code. Until this is fixed, treat this contract as having no stable admin and
   do not deploy it with real authority over anything.
2. **The cooldown guard protects one entrypoint.** `set_risk_admin_cooldown`,
   `set_paused`, and `init` are not guarded. The module docs in `src/lib.rs`
   ("called at the top of every state-changing entrypoint") and `src/admin.rs`
   ("injected into every state-changing risk entrypoint") overstate this and
   should be corrected. Functionally the breaker can be disabled between two
   guarded actions.
3. **The cooldown is a rate limit on a bookkeeping call, not on a mutation.**
   `record_risk_admin_action` changes nothing but its own timestamp. Unless the
   caller pairs it with the action it is meant to throttle (§2), it throttles
   nothing. The credit contract's own cooldown (§2) has the stronger property,
   because there the timestamp write and the parameter mutation are the same
   transaction.
4. **`ContractError::Unauthorized` (1) is dead.** It is declared, documented in
   the discriminant table, and asserted in tests, but no code path raises it.
   Either wire it up or remove it — it is currently a misleading member of an
   ABI-stable enum.
5. **`src/views.rs` is not part of this crate.** It is not declared as a module
   in `src/lib.rs` and it imports `crate::risk`, `crate::scoring`,
   `crate::storage::get_credit_line`, and `crate::types::RiskCapabilities` —
   all of which belong to **`creditra-credit`**, not `creditra-risk`. It cannot
   compile here, is never built, and is nevertheless shipped inside the
   published `.crate` (it appears in `cargo package -p creditra-risk --list`).
   `docs/PROTOCOL_SPEC.md` also pointed at it for `risk_capabilities`, which is
   not an entrypoint of either contract. Delete it or move it to the crate it
   belongs to.
6. **No CI job builds or tests `creditra-risk`.** `.github/workflows/ci.yml`
   runs only against `contracts/creditra-credit`; the workspace root is not
   exercised. Everything in §7 is currently a local-only check.
7. **The workspace root `Cargo.toml` lists `contracts/risk` twice** (members
   array), and the root `Cargo.lock` contains a duplicated `creditra-credit`
   package entry, which makes every workspace-level `cargo` invocation fail to
   parse the lockfile.
8. **The crate description mentions a campaign** ("GrantFox FWC26 campaign")
   rather than the contract's function. Cosmetic, but it is the text that
   surfaces on the registry.
9. **The `instrument`-gated test targets do not compile, and
   `cargo clippy --all-targets -- -D warnings` is red for this crate.**
   `tests/proptest.rs` reads `.topics` / `.data` from the event tuple
   (`tests/proptest.rs:100`, `:105`) and has a type mismatch at `:624`;
   `tests/events.rs` trips `clippy::clone_on_copy` and
   `clippy::bool_assert_comparison`. `cargo test -p creditra-risk` without
   `--features instrument` passes all 29 tests, which is why this went
   unnoticed — no CI job runs any of it (§8, gap 6).

---

## 9. Related documents

- [`README.md`](../../README.md) — repository map and crate table.
- [`docs/INDEX.md`](../../docs/INDEX.md) — documentation index.
- [`docs/PROTOCOL_SPEC.md`](../../docs/PROTOCOL_SPEC.md) — credit-contract entrypoint surface.
- [`docs/threat-model.md`](../../docs/threat-model.md) — authorization matrix.
- [`fuzz/README.md`](./fuzz/README.md) — fuzz target and invariants.
