# Creditra Risk-Pricing Algorithm — In Depth

This document is the formal description of the on-chain risk-pricing function:
how Creditra computes a credit limit and an interest rate for each borrower,
how interest accrues, how delinquency is priced, and how recovery is settled
in a default auction. Every formula here is implemented in the source under
`contracts/credit/src/{risk.rs,accrual.rs,math_utils.rs,lifecycle.rs}` and is
covered by the test files enumerated in
`contracts/credit/tests/{accrual_overflow_audit.rs,risk_formula_tests.rs,...}`.

Companion docs: `docs/risk-based-rate-formula.md` (terse normative reference),
`docs/interest-accrual.md` (accrual normative reference),
`WHITEPAPER.md` (protocol-level model).

---

## 1. Inputs the Function Sees

| Input | Type | Range | Source |
|---|---|---|---|
| Risk score $k$ | `u32` | `[0, MAX_RISK_SCORE=100]` | Off-chain scorer pushed via `update_risk_parameters` (`risk.rs:207`) |
| Credit limit $\ell$ | `i128` | `[MinCreditLimit, MaxCreditLimit]` | Off-chain scorer / admin policy |
| Rate config $(b, s, r_{\min}, r_{\max})$ | `RateFormulaConfig` | each `u32` in `[0, 10_000]`, `r_{\min} \leq r_{\max} \leq 10\,000` | `set_rate_formula_config` (`lib.rs:1159`) |
| Per-borrower floor $r_{\text{floor}}$ | `Option<u32>` | `[0, 10_000]` | `set_borrower_rate_floor` (`lib.rs:578`) |
| Per-borrower ceiling $r_{\text{ceiling}}$ | `Option<u32>` | `[0, 10_000]` and `floor <= ceiling` when both are set | `set_borrower_rate_ceiling` (`lib.rs:775`) |
| Rate-change config $(\Delta r_{\max}, \tau_{\min})$ | `RateChangeConfig` | `bps, seconds` | `set_rate_change_limits` (`lib.rs:569`) |
| Penalty surcharge $\rho$ | `u32` | `[0, 10_000]` | `set_penalty_surcharge_bps` (`lib.rs:587`) |
| Grace period $(T_g, m, r_g)$ | `GracePeriodConfig` | $T_g$ in seconds, mode FullWaiver/ReducedRate, $r_g$ in bps | `set_grace_period_config` (`lib.rs:646`) |
| Utilization $u$ | `i128` | `[0, \ell]` | mutated on draw/repay |
| Accrued interest $I$ | `i128` | `[0, ...)` | mutated on accrual fold |
| Last accrual timestamp $t_{\text{last}}$ | `u64` | unix seconds | updated only when $\Delta I > 0$ |

The on-chain function is therefore deterministic in
$(k, \ell, b, s, r_{\min}, r_{\max}, r_{\text{floor}}, r_{\text{ceiling}}, \rho, T_g, m, r_g, u, I, t_{\text{last}}, t_{\text{now}})$
— there is no hidden state.

---

## 2. The Rate Function

### 2.1 Formal definition

Let

$$
\mathrm{clamp}(x, a, b) = \min(\max(x, a), b)
$$

The on-chain rate function `compute_rate_from_score`
(`contracts/credit/src/risk.rs:77`) is:

$$
r(k) = \mathrm{clamp}\Big(b + k \cdot s, \; r_{\min}, \; \min(r_{\max}, R_{\text{cap}})\Big)
$$

where $R_{\text{cap}} = \text{MAX\_INTEREST\_RATE\_BPS} = 10\,000$
(`risk.rs:24`).

The implementation uses **saturating** arithmetic on the `u32`
multiplication, so a misconfigured `b + 100·s > u32::MAX` saturates rather
than overflowing. The clamp then brings it back into the configured range.

Equivalent pseudocode:

```rust
pub fn compute_rate_from_score(cfg: &RateFormulaConfig, k: u32) -> u32 {
    let raw   = cfg.base_rate_bps.saturating_add(k.saturating_mul(cfg.slope_bps_per_score));
    let upper = cfg.max_rate_bps.min(MAX_INTEREST_RATE_BPS);
    raw.clamp(cfg.min_rate_bps, upper)
}
```

### 2.2 Per-borrower floor and ceiling

After the formula computes $r(k)$, an optional per-borrower floor
$r_{\text{floor}}$ and ceiling $r_{\text{ceiling}}$ are applied:

$$
r_{\text{eff}}(k, \text{borrower}) = \min\Big(\max\big(r(k), \; r_{\text{floor}}(\text{borrower}) \big), \; r_{\text{ceiling}}(\text{borrower})\Big)
$$

The floor is stored under `DataKey::RateFloorBps(Address)` (Persistent,
`contracts/credit/src/storage.rs:357`). Use cases: a borrower in a higher-risk
jurisdiction, or one with a sticky penalty history, can be assigned a hard
minimum rate that overrides a favorable formula.

The ceiling is stored under `DataKey::RateCeilingBps(Address)` (Persistent).
It caps that borrower's manual or formula-derived rate before rate-change
guardrails run. The admin setters reject inconsistent bounds in either
direction: a ceiling below an existing floor, or a floor above an existing
ceiling, reverts with `ContractError::RateTooHigh`.

When either bound is unset, that side of the clamp is skipped.

### 2.3 Rate-change cap

`update_risk_parameters` (`risk.rs:207`) further constrains rate changes via
`RateChangeConfig`:

$$
|r_{\text{new}} - r_{\text{old}}| \leq \Delta r_{\max} \quad \text{AND} \quad t_{\text{now}} - t_{\text{last\_rate\_update}} \geq \tau_{\min}
$$

Violations revert:

- Magnitude breach → `ContractError::RateTooHigh = 8`
- Cadence breach → `ContractError::TimestampRegression = 33`

`last_rate_update_ts` is only advanced when a rate actually changes, so a
no-op `update_risk_parameters` does not reset the cadence clock.

### 2.4 Worked numerical example — rate

Configure:

```
base_rate_bps = 200       // 2.00 % floor
slope_bps_per_score = 50  // 0.5 % per score point
min_rate_bps = 200        // 2.00 %
max_rate_bps = 5000       // 50.00 %
```

For $k \in \{0, 25, 50, 75, 100\}$:

| $k$ | $b + k \cdot s$ | clamp to $[200, 5000]$ | $r(k)$ (APR) |
|---|---|---|---|
| 0   | 200       | 200       | 2.00 %  |
| 25  | 1450      | 1450      | 14.50 % |
| 50  | 2700      | 2700      | 27.00 % |
| 75  | 3950      | 3950      | 39.50 % |
| 100 | 5200      | 5000      | 50.00 % (clamped) |

A borrower with $r_{\text{floor}} = 1000$ at $k=0$ would see $r_{\text{eff}}
= \max(200, 1000) = 1000$ (10.00 %).

A borrower with $r_{\text{ceiling}} = 4000$ at $k=100$ would see
$r_{\text{eff}} = \min(5000, 4000) = 4000$ (40.00 %), even though the formula
would otherwise clamp at 50.00 %.

This example is the canonical test fixture in `tests/risk_formula_tests.rs`.

### 2.5 Worked numerical example — rate-change magnitude cap

`update_risk_parameters` enforces `|r_new - r_old| ≤ Δr_max` when
`RateChangeConfig` is active (`risk.rs:340-355`).

Configure `max_rate_change_bps = 200` (2.00 % per update).

| Scenario | Old rate | New formula rate | Delta | Permitted? |
|---|---|---|---|---|
| Gradual increase | 500 bps (5.00 %) | 650 bps (6.50 %) | 150 bps | Yes (150 ≤ 200) |
| Sharp increase | 500 bps (5.00 %) | 1 000 bps (10.00 %) | 500 bps | No (500 > 200) → revert `RateTooHigh` |
| Gradual decrease | 3 000 bps (30.00 %) | 2 850 bps (28.50 %) | 150 bps | Yes (150 ≤ 200) |
| Sharp decrease | 3 000 bps (30.00 %) | 2 500 bps (25.00 %) | 500 bps | No (500 > 200) → revert `RateTooHigh` |
| No change | 1 500 bps | 1 500 bps | 0 bps | Skipped (no delta check) |

The delta check uses `u32::abs_diff` (`risk.rs:343`), so the cap is
**symmetric** — it applies equally to rate increases and decreases.

Tested in `tests/state_transition_invariants.rs` (magnitude) and
`risk_formula_tests.rs:514-535` (formula + rate-change limits).

### 2.6 Worked numerical example — rate-change cadence cap

`update_risk_parameters` also enforces a minimum interval between rate
changes when `rate_change_min_interval > 0` (`risk.rs:348-355`).

Configure `max_rate_change_bps = 500`, `rate_change_min_interval = 86 400`
(1 day).

| Event | Timestamp | Rate change | Permitted? | Reason |
|---|---|---|---|---|
| Open line | t=0 | 500 → 500 bps | N/A | No prior rate |
| First update | t=3 600 (1 hr) | 500 → 700 bps (Δ=200) | Yes | No prior rate change |
| Second update | t=3 600 + 43 200 (12 hr) | 700 → 900 bps (Δ=200) | No | 43 200 s < 86 400 s → revert `TimestampRegression` |
| Third update | t=3 600 + 86 400 (1 day) | 700 → 900 bps (Δ=200) | Yes | 86 400 s ≥ 86 400 s |
| Fourth update | t=3 600 + 86 400 + 1 | 900 → 800 bps (Δ=100) | Yes | ≥86 400 s elapsed |

The cadence check is **skipped** when `rate_change_min_interval == 0`
(no minimum) or when `last_rate_update_ts == 0` (no prior rate change,
`risk.rs:348`). `last_rate_update_ts` is only advanced when the rate
*actually changes*, so a no-op update does not reset the cadence clock.

Tested in `tests/monotonic_timestamps.rs`.

### 2.7 Worked numerical example — per-borrower floor/ceiling stacking

After the formula computes the rate, two optional per-borrower overrides
apply in sequence (`risk.rs:328-338`):

```
r_mid  = max(r_formula, r_floor)       // floor applied first
r_eff  = min(r_mid,    r_ceiling)      // ceiling applied second
```

Example with `RateFormulaConfig(200, 50, 200, 5 000)`:

| Scenario | $k$ | $r_{\text{formula}}$ | Floor (bps) | Ceiling (bps) | $r_{\text{eff}}$ |
|---|---|---|---|---|---|
| No overrides | 25 | 1 450 | — | — | 1 450 bps (14.50 %) |
| Floor only | 0 | 200 | 1 000 | — | 1 000 bps (10.00 %) |
| Ceiling only | 75 | 3 950 | — | 2 500 | 2 500 bps (25.00 %) |
| Floor + ceiling sandwich | 50 | 2 700 | 3 000 | 4 000 | 3 000 bps (30.00 %) |
| Ceiling below floor (rejected) | 50 | 2 700 | 3 000 | 2 500 | Rejected at config-set time (`RateTooHigh`) |

The contract rejects inconsistent borrower-specific bounds in either
direction: `ceiling < floor` when setting a ceiling and `floor > ceiling`
when setting a floor. That prevents a misconfigured admin from creating an
unresolvable ordering. Tested in `tests/borrower_rate_floor.rs` and
`tests/borrower_rate_ceiling.rs`.

---

## 3. The Credit-Limit Function

In v1 the on-chain function accepts an admin-supplied `credit_limit` and
validates it. The off-chain composition is:

$$
\ell(\text{borrower}) = \mathrm{clip}\Big(\ell_{\text{base}} \cdot f(k, h, a, \alpha), \; \ell_{\text{min}}, \; \ell_{\text{max}}\Big)
$$

where:

- $\ell_{\text{base}}$ is a per-protocol base limit (e.g. 100 XLM
  equivalent) set by policy.
- $f(k, h, a, \alpha)$ is the off-chain multiplier as a function of the
  score $k$, history vector $h$ (repayments, recoveries), attestation
  bundle $a$, and the historical default recovery probability $\alpha$.
- $\ell_{\text{min}}, \ell_{\text{max}}$ are the on-chain bounds set by
  `set_credit_limit_bounds(min, max)` (`lib.rs:862`, validated in
  `lifecycle.rs:78-145`).

The on-chain validation enforces:

$$
\ell_{\text{min}} \leq \ell \leq \ell_{\text{max}}, \quad \ell \geq 0
$$

Violations revert `LimitOutOfBounds = 34` or `NegativeLimit = 7`.

### 3.1 The Restricted promotion path

`update_risk_parameters` reduces the limit relative to current utilization.
If the new $\ell < u$, the contract does **not** revert — it sets
`status = Restricted` (`risk.rs:207`). The borrower:

- Cannot draw further (the limit check fails).
- Can still repay normally.
- On repayments that reduce $u$ below the new $\ell$, the line auto-cures
  back to `Active`.

The off-chain protocol design intent is that a scorer who detects increased
risk (e.g. a counterparty default in the borrower's transaction graph) can
unilaterally tighten the credit line without forcing an immediate default —
the line is rate-limited rather than terminated.

### 3.2 Global exposure cap

`MaxTotalExposure` (`lib.rs:827`) is an absolute ceiling:

$$
\sum_{i} u_i + a \leq \text{MaxTotalExposure} \quad \text{(checked on every draw)}
$$

Violations revert `ExposureCapExceeded = 31`. This is the protocol-wide
circuit breaker that bounds total loss from a misbehaving scorer or
misconfigured limits.

### 3.3 Per-borrower utilization cap

`UtilizationCapBps(borrower)` (`storage.rs:364`) is the per-borrower draw
ratio cap, applied at draw time as:

$$
u + a \leq \frac{\ell \cdot \text{cap\_bps}}{10\,000}
$$

`cap_bps = 0` removes the cap. Documented in `docs/utilization-cap.md`.

---

## 4. Interest Accrual

### 4.1 The fold (live formula)

`apply_accrual` (`contracts/credit/src/accrual.rs:87`) is invoked at the
head of every state-mutating entrypoint. The pure-math part is in
`math_utils::prorate_interest`
(`contracts/credit/src/math_utils.rs:244`):

$$
\Delta I = \left\lfloor \frac{u \cdot r_{\text{eff}} \cdot \Delta t}{10\,000 \cdot Y} \right\rfloor
$$

where:

- $u$ is `utilized_amount` at the start of the fold
- $r_{\text{eff}}$ is the **effective** rate in bps (see §4.2)
- $\Delta t = t_{\text{now}} - t_{\text{last}}$ in seconds
- $Y$ = `SECONDS_PER_YEAR = 31_557_600` (Julian year,
  `math_utils.rs:60`)
- Floor rounding via `Rounding::Floor` (`math_utils.rs:76`)

Capitalization:

$$
u' = u + \Delta I, \quad I_{\text{accrued}}' = I_{\text{accrued}} + \Delta I
$$

$t_{\text{last}}' = t_{\text{now}}$ **only if** $\Delta I > 0$. This avoids
the silent-zeroing pathology where a sub-tick call zeroes the time delta
without ever charging interest.

If $u = 0$ or $t_{\text{now}} \leq t_{\text{last}}$, the fold is a no-op
and $t_{\text{last}}$ is preserved exactly.

### 4.2 The three branches of effective rate

`apply_accrual` chooses $r_{\text{eff}}$ based on line state and delinquency:

#### Branch A — Active line, current

$$
r_{\text{eff}} = r
$$

where $r$ is `interest_rate_bps`. Standard case.

#### Branch B — Active line, delinquent

If `is_delinquent(borrower) == true`
(`query.rs:57`, which checks $t_{\text{now}} > \text{next\_due\_ts} +
\text{grace}$ saturating-add), the surcharge $\rho$ applies:

$$
r_{\text{eff}} = \min(r + \rho, \; R_{\text{cap}})
$$

The first delinquent accrual emits `PenaltyRateEnteredEvent` (topic
`("credit","pen_enter")`); the first non-delinquent accrual after delinquency
emits `PenaltyRateExitedEvent` (`("credit","pen_exit")`). Source:
`accrual.rs`, events in `events.rs:278,296`.

#### Branch C — Suspended line with grace policy

If the line is `Suspended` and a `GracePeriodConfig { T_g, m, r_g }` is set,
$\Delta t$ is split:

$$
\Delta t_g = \min(\Delta t, T_g), \quad \Delta t_p = \Delta t - \Delta t_g
$$

Then:

$$
\Delta I = \begin{cases}
\mathrm{prorate}(u, r, \Delta t_p) & \text{if } m = \text{FullWaiver} \\
\mathrm{prorate}(u, r_g, \Delta t_g) + \mathrm{prorate}(u, r, \Delta t_p) & \text{if } m = \text{ReducedRate}
\end{cases}
$$

`FullWaiver` is the default (`GraceWaiverMode::FullWaiver = 0`,
`types.rs:267`).

### 4.3 Why simple interest, capitalized at mutation?

Two reasons:

1. **Gas predictability.** Per-call accrual cost is $O(1)$ — one mul, one
   div, one storage write per affected key. There is no on-chain compounding
   loop. A borrower who never touches the line for 5 years incurs the same
   accrual cost as one who repays daily.

2. **Floor rounding favors the borrower.** Every $\Delta I$ rounds down.
   The aggregate bias is against protocol revenue, never against borrower
   balance. This is intentional — it makes the contract trivially safe to
   audit for rounding-direction attacks.

The model in `docs/interest-accrual.md` notes this as a "checkpoint-on-
mutation" design and contrasts it with the per-block accrual of
Aave/Compound (which requires a separate `accrue` keeper or per-call
state mutation on every read).

### 4.4 Worked numerical example — accrual

A borrower draws **1 000 XLM** (with XLM as a 7-decimal asset, so the
on-chain `i128` value is $1\,000 \cdot 10^7 = 10\,000\,000\,000$ stroops).
Their rate is $r = 1\,500$ bps (15.00 % APR).

After **30 days** (Δt = 2 592 000 seconds):

$$
\Delta I = \left\lfloor \frac{10\,000\,000\,000 \cdot 1\,500 \cdot 2\,592\,000}{10\,000 \cdot 31\,557\,600} \right\rfloor
$$

Numerator: $10^{10} \cdot 1.5 \cdot 10^3 \cdot 2.592 \cdot 10^6
 = 3.888 \cdot 10^{19}$
Denominator: $10^4 \cdot 3.15576 \cdot 10^7 = 3.15576 \cdot 10^{11}$
Quotient: $3.888 \cdot 10^{19} / 3.15576 \cdot 10^{11}
 \approx 1.23204 \cdot 10^{8}$

$$
\Delta I \approx 123\,205\,479 \text{ stroops} \approx 12.32 \text{ XLM}
$$

This is the realized 30-day interest at 15 % APR on 1 000 XLM, which is the
expected result (15 % × 30/365.25 × 1 000 ≈ 12.32 XLM). The on-chain stored
value is exactly $\lfloor 38\,880\,000\,000 / 315.576 \rfloor$, which the
test fixture in `tests/accrual_overflow_audit.rs` confirms.

If the borrower is now delinquent (past `next_due_ts + grace`) with a
$\rho = 500$ bps surcharge, the effective rate becomes $r_{\text{eff}} =
\min(1\,500 + 500, 10\,000) = 2\,000$ bps (20 % APR). The 30-day Δ$I$
recomputes to ~16.43 XLM, and a `PenaltyRateEnteredEvent` is emitted on
the first such accrual.

### 4.5 Worked numerical example — grace + ReducedRate

Same 1 000 XLM at 15 % APR, but the line is `Suspended` and the borrower
calls `repay_credit` **45 days** after suspension. `GracePeriodConfig` is
set with $T_g = 30\,\text{d}$, $m = \text{ReducedRate}$,
$r_g = 500$ bps (5 %).

Split:

- $\Delta t_g = \min(45\,\text{d}, 30\,\text{d}) = 30\,\text{d} = 2\,592\,000$ s
- $\Delta t_p = 45 - 30 = 15\,\text{d} = 1\,296\,000$ s

Grace-period accrual at 5 % APR for 30 days on 1 000 XLM:
$\approx 4.10$ XLM

Post-grace accrual at 15 % APR for 15 days:
$\approx 6.16$ XLM

Total $\Delta I \approx 10.27$ XLM.

Compare to no-grace accrual (45 days at 15 %): $\approx 18.48$ XLM.

The grace mode reduced the realized interest by ~8.20 XLM.

### 4.6 Worked numerical example — multi-year simple interest (no compounding)

Creditra uses simple interest capitalized at mutation. There is no automatic
compounding loop. A borrower who draws and never touches the line for 5 years
accrues interest linearly, *not* exponentially.

Take the §4.4 scenario (1 000 XLM at 15.00 % APR). Compare simple vs.
compound interest over multiple years:

| Year | Simple interest accrued (cumulative) | Compound interest (annual compounding, cumulative) |
|---|---|---|
| 0 | — | — |
| 1 | 12.32 XLM | 12.32 XLM |
| 2 | 24.65 XLM | 26.49 XLM |
| 3 | 36.97 XLM | 42.59 XLM |
| 4 | 49.30 XLM | 60.99 XLM |
| 5 | 61.62 XLM | 82.23 XLM |

After 5 years the simple-interest borrower owes approximately **1 061.62
XLM**, while a compound-interest equivalent would owe approximately
**1 082.23 XLM** — a difference of ~20.61 XLM in the borrower's favor.

The simple-interest design:
- **Benefits consistent repayers.** A borrower who repays frequently
  minimizes the principal on which future interest accrues, without being
  penalised by a compounding treadmill.
- **Eliminates the compounding oracle problem.** No need for keeper bots
  to periodically compound — the contract charges interest only when a
  mutation occurs.
- **Is trivially auditable.** Every $\Delta I$ is a single
  `mul_div(utilized, rate * Δt, 10_000 * Y, Floor)` call. There is no
  compounding loop to reason about.

The multi-period test in `contracts/credit/src/accrual_tests.rs:100-127`
demonstrates the additive property across two six-month windows.

### 4.7 Worked numerical example — sub-tick dust accumulation

Floor rounding (`Rounding::Floor` in `prorate_interest`,
`math_utils.rs:244`) means very short time windows produce $\Delta I = 0$.
However, the contract **advances** `last_accrual_ts` even on a zero-interest
fold (provided $u > 0$ and $t_{\text{now}} > t_{\text{last}}$,
`accrual.rs` design). This means fractional "dust" is not accumulated
indefinitely — the contract trades sub-tick precision for gas efficiency.

Example: 1 000 000 XLM at 500 bps (5.00 % APR).

| Δt | $\Delta I$ formula | $\Delta I$ (stroops) | Note |
|---|---|---|---|
| 1 s | $10^{13} \cdot 500 \cdot 1 / (10^4 \cdot 3.15576 \cdot 10^7)$ | 0 | Sub-tick: rounds to 0 |
| 3 600 s (1 hr) | $10^{13} \cdot 500 \cdot 3\,600 / (10^4 \cdot 3.15576 \cdot 10^7)$ | 57 035 | ~0.00057 % of principal |
| 86 400 s (1 day) | $10^{13} \cdot 500 \cdot 86\,400 / (10^4 \cdot 3.15576 \cdot 10^7)$ | 1 368 847 | ~0.0137 % of principal |
| 604 800 s (1 wk) | $10^{13} \cdot 500 \cdot 604\,800 / (10^4 \cdot 3.15576 \cdot 10^7)$ | 9 581 932 | ~0.096 % of principal |
| 31 557 600 s (1 yr) | $10^{13} \cdot 500 \cdot 31\,557\,600 / (10^4 \cdot 3.15576 \cdot 10^7)$ | 500 000 000 | Exactly 5.00 % |

A 1-second accrual on any realistic principal produces $\Delta I = 0$. This
is **not a bug** — it is the consequence of integer arithmetic with a
$Y = 31\,557\,600$-second denominator. The contract compensates by
advancing `last_accrual_ts` unconditionally (when $u > 0$), so the lost
dust is at most one second's worth of interest, never more. Over the
lifetime of a 1 000 000 XLM line at 5.00 %, the maximum dust lost is:

$$
\text{max\_dust} = \left\lfloor \frac{10^{13} \cdot 500 \cdot 1}{10^4 \cdot 3.15576 \cdot 10^7} \right\rfloor
= 0 \text{ stroops}
$$

...but only if the line is touched every second, which is impractical.
In practice, real-world usage patterns produce $\Delta I > 0$ on every
meaningful touch.

Tested in `tests/accrual_overflow_audit.rs` and
`contracts/credit/src/accrual_tests.rs:173-190`.

---

## 5. Repayment Allocation

`repay_credit` (`lib.rs:437-556`) allocates a repayment $a$ as
**interest-first, then principal, then protocol fee on the interest portion**:

$$
\begin{aligned}
a_{\text{eff}} &= \min(a, u) \\
a_I &= \min(a_{\text{eff}}, I_{\text{accrued}}) \\
a_P &= a_{\text{eff}} - a_I \\
\text{fee} &= \left\lfloor \frac{a_I \cdot \phi}{10\,000} \right\rfloor \\
a_{\text{reserve}} &= a_{\text{eff}} - \text{fee}
\end{aligned}
$$

where $\phi = \text{ProtocolFeeBps} \leq \text{MAX\_PROTOCOL\_FEE\_BPS} =
1\,000$ (`lib.rs:63`). The fee is `transfer_from(borrower, contract, fee)`
(accumulates in `TreasuryBalance`); the reserve portion is
`transfer_from(borrower, liquidity_source, a_reserve)`. The admin later
calls `withdraw_treasury(admin)` (`lib.rs:770`) to drain accumulated fees
to `TreasuryAddress`.

### 5.1 Why interest-first?

If repayments were applied principal-first, a borrower making the minimum
payment that covers interest would never amortize. Interest-first is the
amortization-honest split that gives the borrower deterministic principal
reduction per repayment dollar past the interest portion.

### 5.2 Why fee on interest only?

Charging the fee on principal would make the protocol fee an additional
borrowing cost, distinct from the interest rate. Fee-on-interest is
algebraically equivalent to: "the protocol takes a fixed cut of the
interest revenue". The borrower sees only the headline rate
$r$ as their cost; the fee is a partition of the protocol's revenue, not
a separate charge. This is also why fee accounting is independent of
principal repayments — `FeeAccruedEvent` is emitted only when
$a_I > 0$ (`events.rs:170`).

### 5.3 Worked numerical example — repayment

Continuing the §4.4 scenario. After 30 days the borrower owes:
- Principal: 1 000 XLM
- Accrued interest: 12.32 XLM
- Total utilized (post-capitalization): 1 012.32 XLM

The borrower repays $a = 100$ XLM. Suppose $\phi = 200$ bps (2 % fee on
interest).

$a_{\text{eff}} = \min(100, 1\,012.32) = 100$ XLM
$a_I = \min(100, 12.32) = 12.32$ XLM
$a_P = 100 - 12.32 = 87.68$ XLM
$\text{fee} = \lfloor 12.32 \cdot 200 / 10\,000 \rfloor \approx 0.246$ XLM
$a_{\text{reserve}} = 100 - 0.246 = 99.754$ XLM

State after repay:
- Treasury balance: $+0.246$ XLM
- Reserve receives: $99.754$ XLM (out of borrower's allowance)
- Accrued interest: $0$
- Utilized: $1\,012.32 - 100 = 912.32$ XLM
- Schedule's `next_due_ts` advanced by floor($100 / \text{amount\_per\_period}$)
  installments.

---

## 6. Default & Dutch-Auction Recovery

### 6.1 Default trigger

`default_credit_line(borrower)` (`lifecycle.rs:450`) transitions the line
to `Defaulted`. The accrual is applied before the transition so the
recorded `utilized_amount` is the realized debt at default. An event
`("credit","liq_req")` is emitted with the outstanding amount
(`events.rs:236`):

```
DefaultLiquidationRequestedEvent {
    borrower: Address,
    utilized_amount: i128,   // the debt
}
```

Off-chain orchestrator listens for this topic and constructs an auction.

### 6.2 English auction (default mode)

Minimum next bid:

$$
\text{min\_next\_bid} = \max\Big( \lceil \text{highest\_bid} \cdot (1 + \mu/10\,000) \rceil, \; \text{highest\_bid} + 1 \Big)
$$

where $\mu$ is `min_increment_bps` (`init_auction` parameter, capped at
10 000). The `+1` floor prevents zero-increment grief at very small bids.

Refund of previous bidder is atomic with new bid record, under the
reentrancy guard (`storage.rs:316`, `lib.rs` `place_bid` English branch).

Anti-snipe: see `WHITEPAPER.md` §6.3 — documented in PR #430 but not
active in the live `place_bid` path; tracked as a known gap.

### 6.3 Dutch auction

`AuctionMode::Dutch` with init params `dutch_start_price = p_0`,
`dutch_floor_price = p_f`, plus optional `dutch_decay` and
`dutch_step_count`.

- `dutch_decay = Linear` (or omitted) keeps the original linear decay:

$$
p(t) = p_0 - (p_0 - p_f) \cdot \frac{\min(t, T)}{T}, \quad T = \text{end\_time} - \text{start\_time}
$$

- `dutch_decay = Stepped` splits the same total drop into
  `dutch_step_count` equal time buckets and reprices only at bucket
  boundaries. `dutch_step_count` is required and must be greater than zero.

A bid $a$ qualifies if $a \geq p(t) \land a \geq \text{min\_bid}$. The first
qualifying bid atomically flips the auction to `Closed` and records the
winner.

#### 6.3.1 Worked example — Dutch curve

Auction with $p_0 = 1\,200$ XLM, $p_f = 600$ XLM, $T = 3\,600$ seconds
(1 hour), $\text{min\_bid} = 500$ XLM.

| $t$ (seconds elapsed) | $p(t)$ |
|---|---|
| 0     | 1 200 XLM |
| 600   | 1 100 XLM |
| 1 200 | 1 000 XLM |
| 1 800 | 900 XLM   |
| 2 400 | 800 XLM   |
| 3 000 | 700 XLM   |
| 3 600 | 600 XLM   |
| 4 200 | 600 XLM (clamped to floor) |

A bid of 1 050 XLM at $t = 600$ wins (since $1\,050 \geq 1\,100$ is false →
bid is **rejected** as `BidTooLow = 7`). A bid of 1 100 XLM at $t = 600$
wins. The qualifying check is strict: $a \geq p(t)$.

### 6.4 Settlement back into the credit contract

After the auction closes, admin calls
`settle_default_liquidation(borrower, recovered_amount, settlement_id, oracle_price)`
on the credit contract (`lib.rs:953`). The cross-contract call to the
auction's `settle_default_liquidation(settlement_id, credit_addr, borrower)`
returns `highest_bid: i128`, and the credit contract asserts this equals
the admin-supplied `recovered_amount` (else `InvalidAmount`).

Accounting:

$$
\begin{aligned}
\text{interest\_settled} &= \min(\text{recovered\_amount}, I_{\text{accrued}}) \\
\text{principal\_settled} &= \text{recovered\_amount} - \text{interest\_settled} \\
u' &= u - \text{recovered\_amount} \\
I_{\text{accrued}}' &= I_{\text{accrued}} - \text{interest\_settled}
\end{aligned}
$$

If $u' = 0$, status becomes `Closed` and the repayment schedule is cleared.
The settlement event `DefaultLiquidationSettledEvent` (topic
`("credit","liq_setl")`) records the full breakdown for the indexer.

### 6.5 Recovery rate accounting

The protocol's empirical recovery rate for a line is:

$$
\eta(\text{line}) = \frac{\sum \text{recovered\_amount}}{\text{utilized\_amount at default}}
$$

This is computable directly from on-chain events:
$\text{utilized\_amount}$ comes from the `("credit","defaulted")` event;
$\sum \text{recovered\_amount}$ is the sum over all
`("credit","liq_setl")` events for that borrower. The scorer should feed
this back into future $f(k, h, a, \alpha)$ computations.

### 6.6 Worked numerical example — settlement allocation

A borrower defaults with the following state (`lifecycle.rs:450`):

- Principal drawn: 4 500 XLM
- Accrued interest: 500 XLM
- Utilized at default: $u = 5\,000$ XLM
- $I_{\text{accrued}} = 500$ XLM

The Dutch auction resolves with a winning bid of **3 000 XLM**. Admin calls
`settle_default_liquidation` (`lib.rs:953`). The settlement math
(§6.4) gives:

$$
\begin{aligned}
\text{interest\_settled} &= \min(3\,000, 500) = 500\ \text{XLM} \\
\text{principal\_settled} &= 3\,000 - 500 = 2\,500\ \text{XLM} \\
u' &= 5\,000 - 3\,000 = 2\,000\ \text{XLM} \\
I_{\text{accrued}}' &= 500 - 500 = 0\ \text{XLM}
\end{aligned}
$$

Post-settlement state:

| Field | Before | After |
|---|---|---|
| `utilized_amount` | 5 000 XLM | 2 000 XLM |
| `accrued_interest` | 500 XLM | 0 XLM |
| Status | Defaulted | Defaulted (still outstanding) |

Since $u' > 0$, the line remains `Defaulted` and may be re-auctioned for
the remaining 2 000 XLM. The `DefaultLiquidationSettledEvent` records:
`recovered_amount = 3 000`, `interest_settled = 500`,
`principal_settled = 2 500`, `remaining = 2 000`.

**Partial recovery scenario:** If the auction recovers **6 000 XLM** (above
the full debt):

$$
\begin{aligned}
\text{interest\_settled} &= \min(6\,000, 500) = 500 \\
\text{principal\_settled} &= 6\,000 - 500 = 5\,500 \\
u' &= 5\,000 - 6\,000 = \max(5\,000 - 6\,000, 0) = 0
\end{aligned}
$$

The surplus 500 XLM is *not* refunded to the defaulting borrower — the
recovery auction is an enforced liquidation, not a voluntary sale. The
contract caps the recovered amount at `utilized_amount` during settlement
validation. Status transitions to `Closed` and the repayment schedule is
cleared.

Tested in `tests/credit_auction_e2e.rs` and
`tests/default_liquidation_settled_event.rs`.

### 6.7 Worked numerical example — recovery rate accounting

The empirical recovery rate $\eta$ (§6.5) aggregates across a borrower's
default-settlement events.

Borrower Alice has:

| Default event | $u$ at default | Settlement events | $\sum \text{recovered}$ | $\eta$ |
|---|---|---|---|---|
| 2026-01-15 | 5 000 XLM | 3 000 XLM (Jan), 1 500 XLM (Feb) | 4 500 XLM | $4\,500 / 5\,000 = 90.0\,\%$ |
| 2026-06-01 | 2 000 XLM | 800 XLM (Jun) | 800 XLM | $800 / 2\,000 = 40.0\,\%$ |
| **Lifetime** | **7 000 XLM** | **5 300 XLM** | **5 300 XLM** | **$5\,300 / 7\,000 = 75.7\,\%$** |

The protocol's aggregate $\eta$ is computed across **all** borrowers:

$$
\eta_{\text{protocol}} = \frac{\sum_{\text{all borrowers}} \sum \text{recovered\_amount}}
{\sum_{\text{all borrowers}} \text{utilized\_amount at default}}
$$

This ratio feeds back into the off-chain multiplier
$f(k, h, a, \alpha)$ — higher $\eta$ implies tighter future limits
for the same risk score, because the protocol prices in expected loss.

Every settlement emits `("credit","liq_setl")` with the full breakdown
(`events.rs:236`), making $\eta$ computable from emitted events alone.
An indexer can maintain a materialized view without querying past ledger
state.

Tested in `tests/default_liquidation_settled_event.rs`.

---

## 7. Anti-Snipe Semantics (Spec, Tracked as Open)

The intended anti-snipe behavior, per PR #430, is:

- An **extension window** $W$ before `end_time`.
- A **bid extension** $E$ added to `end_time` if a qualifying bid lands
  inside $W$.

Pseudocode:

```rust
if state.config.end_time - now < ANTI_SNIPE_WINDOW_SECS {
    state.config.end_time = state.config.end_time + ANTI_SNIPE_EXTEND_SECS;
}
```

Live `place_bid` (`gateway-contract/.../lib.rs`) does **not** extend
`end_time` and instead hard-rejects bids when `now >= end_time`. The
extension is documented but not active in this release. See
`docs/SECURITY.md` known gaps (§6.1) and `WHITEPAPER.md` §6.3.

---

## 8. Comparison to Other Protocols' Pricing

| Protocol | Rate determination | Limit determination | Penalty mechanism |
|---|---|---|---|
| Aave v3 | Utilization-curve (`r = r_0 + u·slope1` below kink, kinked slope above) | Fixed LTV × collateral, fixed liquidation threshold | Liquidation bonus to keeper |
| Compound v3 | Utilization-curve | Per-asset risk parameter × collateral | Same |
| MakerDAO Spark | Stability fee (governance vote) | Vault min-collat ratio | Stability fee + auction discount |
| **Creditra** | **`r = clamp(b + k·s, r_min, min(r_max, 10000))` clamped by RateChangeConfig** | **Off-chain score × policy multiplier, clipped to admin bounds** | **Penalty surcharge ρ added to base rate; grace mode; cross-contract auction recovery** |

Key differences:

1. **The borrower's behavior** (the score $k$) is a first-class input to the
   rate, not just to eligibility. A behaving borrower in good standing pays
   less; the rate adjustment is bounded by `RateChangeConfig` to prevent
   shock.
2. **No utilization curve.** Creditra's per-borrower rate is set by score,
   not by aggregate pool utilization. The market-wide utilization signal can
   be folded into the score off-chain if desired.
3. **The recovery rate is empirical**, not assumed. Each settlement
   contributes one data point to the protocol's recovery distribution.
4. **The penalty surcharge has an exit** (`PenaltyRateExitedEvent`). A
   delinquent borrower who cures returns to the base rate — this is the
   on-chain equivalent of a "good standing" credit-card mechanic.

---

## 9. Test Coverage of the Algorithm

| Behavior | Test file |
|---|---|
| `compute_rate_from_score` clamp | `contracts/credit/src/risk_formula_tests.rs` (inline tests) |
| Saturating arithmetic on rate | `risk_formula_tests.rs` |
| Per-borrower rate floor override | `contracts/credit/tests/borrower_rate_floor.rs` |
| Per-borrower rate ceiling interaction | `contracts/credit/tests/borrower_rate_ceiling.rs` |
| Rate-change cap (magnitude) | `tests/state_transition_invariants.rs`, worked example §2.5 |
| Rate-change cap (cadence) | `tests/monotonic_timestamps.rs`, worked example §2.6 |
| Floor-rounded accrual | `tests/accrual_overflow_audit.rs`, inline `accrual_tests.rs` |
| Sub-tick dust rounding | inline `accrual_tests.rs`, worked example §4.7 |
| Multi-year simple-interest accumulation | inline `accrual_tests.rs`, worked example §4.6 |
| Overflow safety | `tests/accrual_overflow_audit.rs` |
| Penalty surcharge entry/exit | `tests/penalty_surcharge.rs` |
| Grace waiver (FullWaiver vs ReducedRate) | `tests/grace_waiver.rs`, worked example §4.5 |
| Grace + ReducedRate split-window | inline `accrual_tests.rs`, worked example §4.5 |
| Restricted-on-limit-decrease | `tests/restricted_status.rs` |
| Interest-first repay allocation | `tests/protocol_fee.rs` |
| Fee accounting (protocol fee) | `tests/protocol_fee.rs` |
| Default → auction → settle flow | `tests/credit_auction_e2e.rs`, worked example §6.6 |
| Settlement replay protection | `tests/default_liquidation_settled_event.rs` |
| Recovery rate accounting | worked example §6.7, computable from events |
| Dutch auction curve | `gateway-contract/.../test.rs`, worked example §6.3.1 |
| Anti-snipe (open gap) | not currently tested in live path |

---

## 10. References

- `contracts/credit/src/risk.rs` — `compute_rate_from_score`, `update_risk_parameters`
- `contracts/credit/src/accrual.rs` — `apply_accrual`, branches
- `contracts/credit/src/math_utils.rs` — `prorate_interest`, `mul_div`, `Rounding`
- `contracts/credit/src/lib.rs` — `repay_credit`, `settle_default_liquidation`
- `contracts/credit/src/lifecycle.rs` — `default_credit_line`,
  `settle_default_liquidation`
- `gateway-contract/contracts/auction_contract/src/lib.rs` — Dutch & English auctions
- `docs/risk-based-rate-formula.md` — normative reference
- `docs/interest-accrual.md`, `docs/interest-accrual-design.md` — accrual references
- `WHITEPAPER.md` — protocol-level model
