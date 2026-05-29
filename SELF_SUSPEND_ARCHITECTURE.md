# Self-Suspend Credit Line - Architecture & Flow Diagrams

## System Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     Creditra Credit Contract                     │
├─────────────────────────────────────────────────────────────────┤
│                                                                   │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │                    Public API (lib.rs)                   │   │
│  ├─────────────────────────────────────────────────────────┤   │
│  │  • open_credit_line()                                    │   │
│  │  • draw_credit()                                         │   │
│  │  • repay_credit()                                        │   │
│  │  • suspend_credit_line()          [Admin Only]           │   │
│  │  • self_suspend_credit_line()     [Borrower Only] ⭐    │   │
│  │  • reinstate_credit_line()        [Admin Only]           │   │
│  │  • close_credit_line()                                   │   │
│  │  • default_credit_line()          [Admin Only]           │   │
│  └─────────────────────────────────────────────────────────┘   │
│                              │                                    │
│                              ▼                                    │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │              Lifecycle Module (lifecycle.rs)             │   │
│  ├─────────────────────────────────────────────────────────┤   │
│  │  • suspend_credit_line()          [Admin Auth]           │   │
│  │  • self_suspend_credit_line()     [Borrower Auth] ⭐    │   │
│  │  • reinstate_credit_line()        [Admin Auth]           │   │
│  │  • close_credit_line()            [Admin/Borrower Auth]  │   │
│  │  • default_credit_line()          [Admin Auth]           │   │
│  └─────────────────────────────────────────────────────────┘   │
│                              │                                    │
│         ┌────────────────────┼────────────────────┐             │
│         ▼                    ▼                    ▼              │
│  ┌──────────┐        ┌──────────┐        ┌──────────┐          │
│  │  Auth    │        │ Storage  │        │  Events  │          │
│  │  Module  │        │  Module  │        │  Module  │          │
│  └──────────┘        └──────────┘        └──────────┘          │
│                                                                   │
└─────────────────────────────────────────────────────────────────┘
```

---

## Self-Suspend Function Flow

```
┌─────────────────────────────────────────────────────────────────┐
│                  self_suspend_credit_line()                      │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
        ┌─────────────────────────────────────────┐
        │  1. Authorization Check                  │
        │     borrower.require_auth()              │
        │                                          │
        │  ✓ Borrower authorized                   │
        │  ✗ Admin → PANIC (auth failure)          │
        │  ✗ Third party → PANIC (auth failure)    │
        └─────────────────────────────────────────┘
                              │
                              ▼
        ┌─────────────────────────────────────────┐
        │  2. Load Credit Line from Storage        │
        │     env.storage().persistent().get()     │
        │                                          │
        │  ✓ Credit line exists                    │
        │  ✗ Not found → PANIC                     │
        └─────────────────────────────────────────┘
                              │
                              ▼
        ┌─────────────────────────────────────────┐
        │  3. Apply Interest Accrual               │
        │     apply_accrual(&env, credit_line)     │
        │                                          │
        │  • Calculates pending interest           │
        │  • Updates utilized_amount               │
        │  • Updates accrued_interest              │
        └─────────────────────────────────────────┘
                              │
                              ▼
        ┌─────────────────────────────────────────┐
        │  4. Validate Status                      │
        │     if status != Active → PANIC          │
        │                                          │
        │  ✓ Active → Continue                     │
        │  ✗ Suspended → PANIC                     │
        │  ✗ Defaulted → PANIC                     │
        │  ✗ Closed → PANIC                        │
        └─────────────────────────────────────────┘
                              │
                              ▼
        ┌─────────────────────────────────────────┐
        │  5. Update Status                        │
        │     credit_line.status = Suspended       │
        │                                          │
        │  • Only status field changes             │
        │  • All other fields preserved            │
        └─────────────────────────────────────────┘
                              │
                              ▼
        ┌─────────────────────────────────────────┐
        │  6. Persist to Storage                   │
        │     env.storage().persistent().set()     │
        │                                          │
        │  • Atomic update                         │
        │  • Overwrites previous state             │
        └─────────────────────────────────────────┘
                              │
                              ▼
        ┌─────────────────────────────────────────┐
        │  7. Emit Event                           │
        │     publish_credit_line_event()          │
        │                                          │
        │  Event: ("credit", "selfsus")            │
        │  Data: borrower, status, limit, rate     │
        └─────────────────────────────────────────┘
                              │
                              ▼
                        ┌─────────┐
                        │ SUCCESS │
                        └─────────┘
```

---

## State Machine Diagram

```
                    ┌──────────────────────────────────────┐
                    │                                      │
                    │         Credit Line States           │
                    │                                      │
                    └──────────────────────────────────────┘

                              ┌─────────┐
                              │ Active  │ ◄─── open_credit_line()
                              └─────────┘
                                   │
                    ┌──────────────┼──────────────┐
                    │              │              │
                    ▼              ▼              ▼
         ┌──────────────┐  ┌──────────────┐  ┌──────────────┐
         │  Suspended   │  │  Defaulted   │  │   Closed     │
         │  (Admin)     │  │  (Admin)     │  │ (Admin/Borr) │
         └──────────────┘  └──────────────┘  └──────────────┘
                    ▲
                    │
                    │ self_suspend_credit_line() ⭐
                    │ (Borrower Only)
                    │
              ┌─────────┐
              │ Active  │
              └─────────┘


Legend:
  ⭐ = New self-suspend feature
  Admin = Requires admin authorization
  Borr = Requires borrower authorization
  Admin/Borr = Either admin or borrower (with conditions)
```

---

## Authorization Matrix

```
┌─────────────────────────────────────────────────────────────────┐
│                    Function Authorization Matrix                 │
├──────────────────────────┬──────────┬──────────┬────────────────┤
│ Function                 │  Admin   │ Borrower │  Third Party   │
├──────────────────────────┼──────────┼──────────┼────────────────┤
│ open_credit_line         │    ✓     │    ✗     │      ✗         │
│ draw_credit              │    ✗     │    ✓     │      ✗         │
│ repay_credit             │    ✗     │    ✓     │      ✗         │
│ suspend_credit_line      │    ✓     │    ✗     │      ✗         │
│ self_suspend_credit_line │    ✗     │    ✓ ⭐  │      ✗         │
│ reinstate_credit_line    │    ✓     │    ✗     │      ✗         │
│ close_credit_line        │    ✓     │  ✓ (*)   │      ✗         │
│ default_credit_line      │    ✓     │    ✗     │      ✗         │
│ update_risk_parameters   │    ✓     │    ✗     │      ✗         │
│ get_credit_line          │    ✓     │    ✓     │      ✓         │
└──────────────────────────┴──────────┴──────────┴────────────────┘

(*) Borrower can close only when utilized_amount == 0
⭐ = New self-suspend feature
```

---

## Post-Suspension Operation Matrix

```
┌─────────────────────────────────────────────────────────────────┐
│          Operations Allowed After Self-Suspension                │
├──────────────────────────┬──────────┬──────────────────────────┤
│ Operation                │ Allowed? │ Notes                     │
├──────────────────────────┼──────────┼──────────────────────────┤
│ draw_credit              │    ✗     │ Blocked while suspended   │
│ repay_credit             │    ✓     │ Always allowed            │
│ self_suspend_credit_line │    ✗     │ Not idempotent            │
│ suspend_credit_line      │    ✗     │ Already suspended         │
│ reinstate_credit_line    │    ✓     │ Admin can restore         │
│ close_credit_line        │    ✓     │ Admin can force-close     │
│ default_credit_line      │    ✓     │ Admin can mark default    │
│ update_risk_parameters   │    ?     │ Implementation dependent  │
│ get_credit_line          │    ✓     │ View always works         │
└──────────────────────────┴──────────┴──────────────────────────┘
```

---

## Test Coverage Map

```
┌─────────────────────────────────────────────────────────────────┐
│                      Test Coverage Matrix                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                   │
│  Authorization Tests (3)                                         │
│  ├─ ✓ Borrower authorized                                        │
│  ├─ ✗ Admin invokes (panic)                                      │
│  └─ ✗ Third party invokes (panic)                                │
│                                                                   │
│  State Machine Tests (5)                                         │
│  ├─ ✓ From Active (success)                                      │
│  ├─ ✗ From Suspended (panic)                                     │
│  ├─ ✗ From Defaulted (panic)                                     │
│  ├─ ✗ From Closed (panic)                                        │
│  └─ ✗ Non-existent line (panic)                                  │
│                                                                   │
│  Functional Tests (5)                                            │
│  ├─ ✗ Draw blocked (expected)                                    │
│  ├─ ✓ Repay allowed (expected)                                   │
│  ├─ ✓ Admin unsuspend (documented)                               │
│  ├─ ✓ Admin close (allowed)                                      │
│  └─ ✓ Utilization preserved                                      │
│                                                                   │
│  Integrity Tests (3)                                             │
│  ├─ ✓ Event emission correct                                     │
│  ├─ ✓ Parameters preserved                                       │
│  └─ ✗ Idempotency (not supported)                                │
│                                                                   │
│  Edge Case Tests (3)                                             │
│  ├─ ✓ Zero utilization                                           │
│  ├─ ✓ Maximum utilization                                        │
│  └─ ✓ Interest accrual applied                                   │
│                                                                   │
│  Total: 19 tests                                                 │
│  Success scenarios: 12                                           │
│  Failure scenarios: 7 (expected panics)                          │
│                                                                   │
└─────────────────────────────────────────────────────────────────┘
```

---

## Data Flow Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│                    Self-Suspend Data Flow                        │
└─────────────────────────────────────────────────────────────────┘

  Borrower                Contract                Storage
     │                       │                       │
     │  self_suspend()       │                       │
     ├──────────────────────>│                       │
     │                       │                       │
     │                       │  require_auth()       │
     │<──────────────────────┤                       │
     │  [Auth Signature]     │                       │
     ├──────────────────────>│                       │
     │                       │                       │
     │                       │  get(borrower)        │
     │                       ├──────────────────────>│
     │                       │                       │
     │                       │  CreditLineData       │
     │                       │<──────────────────────┤
     │                       │                       │
     │                       │  apply_accrual()      │
     │                       │  [Internal]           │
     │                       │                       │
     │                       │  validate_status()    │
     │                       │  [Active check]       │
     │                       │                       │
     │                       │  status = Suspended   │
     │                       │  [Update state]       │
     │                       │                       │
     │                       │  set(borrower, data)  │
     │                       ├──────────────────────>│
     │                       │                       │
     │                       │  [Persisted]          │
     │                       │<──────────────────────┤
     │                       │                       │
     │                       │  emit_event()         │
     │                       │  [Publish]            │
     │                       │                       │
     │  [Success]            │                       │
     │<──────────────────────┤                       │
     │                       │                       │
```

---

## Comparison: Admin Suspend vs Self-Suspend

```
┌─────────────────────────────────────────────────────────────────┐
│         suspend_credit_line vs self_suspend_credit_line          │
├──────────────────────┬──────────────────┬──────────────────────┤
│ Aspect               │ Admin Suspend    │ Self-Suspend ⭐      │
├──────────────────────┼──────────────────┼──────────────────────┤
│ Invoker              │ Admin only       │ Borrower only        │
│ Authorization        │ require_admin()  │ borrower.require()   │
│ Valid from status    │ Active           │ Active               │
│ Result status        │ Suspended        │ Suspended            │
│ Event type           │ "suspend"        │ "selfsus"            │
│ Use case             │ Risk management  │ Voluntary freeze     │
│ Can be overridden    │ By admin         │ By admin             │
│ Idempotent           │ No               │ No                   │
│ Interest accrual     │ Yes              │ Yes                  │
│ Parameter changes    │ Status only      │ Status only          │
└──────────────────────┴──────────────────┴──────────────────────┘
```

---

## Error Handling Flow

```
┌─────────────────────────────────────────────────────────────────┐
│                    Error Handling Paths                          │
└─────────────────────────────────────────────────────────────────┘

  self_suspend_credit_line(borrower)
              │
              ▼
    ┌─────────────────┐
    │ Auth Check      │
    └─────────────────┘
              │
         ┌────┴────┐
         │         │
         ▼         ▼
    ✓ Pass    ✗ Fail ──> PANIC: Authorization failure
         │
         ▼
    ┌─────────────────┐
    │ Load from Store │
    └─────────────────┘
         │
    ┌────┴────┐
    │         │
    ▼         ▼
✓ Found   ✗ Not Found ──> PANIC: "Credit line not found"
    │
    ▼
┌─────────────────┐
│ Apply Accrual   │
└─────────────────┘
    │
    ▼
┌─────────────────┐
│ Status Check    │
└─────────────────┘
    │
┌───┴───┐
│       │
▼       ▼
✓ Active  ✗ Other ──> PANIC: "Only active credit lines can be self-suspended"
│
▼
┌─────────────────┐
│ Update & Save   │
└─────────────────┘
│
▼
┌─────────────────┐
│ Emit Event      │
└─────────────────┘
│
▼
SUCCESS
```

---

## Integration Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                  System Integration Points                       │
└─────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────┐
│                      External Systems                         │
├──────────────────────────────────────────────────────────────┤
│  • Frontend dApp                                             │
│  • Mobile App                                                │
│  • Admin Dashboard                                           │
│  • Monitoring Systems                                        │
└──────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌──────────────────────────────────────────────────────────────┐
│                    Soroban Contract API                       │
├──────────────────────────────────────────────────────────────┤
│  self_suspend_credit_line(borrower: Address)                 │
└──────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌──────────────────────────────────────────────────────────────┐
│                   Contract Internal Modules                   │
├──────────────────────────────────────────────────────────────┤
│  ┌────────────┐  ┌────────────┐  ┌────────────┐            │
│  │   Auth     │  │  Storage   │  │   Events   │            │
│  │  Module    │  │  Module    │  │  Module    │            │
│  └────────────┘  └────────────┘  └────────────┘            │
│                                                              │
│  ┌────────────┐  ┌────────────┐                            │
│  │  Accrual   │  │ Lifecycle  │                            │
│  │  Module    │  │  Module    │                            │
│  └────────────┘  └────────────┘                            │
└──────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌──────────────────────────────────────────────────────────────┐
│                    Soroban Environment                        │
├──────────────────────────────────────────────────────────────┤
│  • Persistent Storage                                        │
│  • Event System                                              │
│  • Authorization Framework                                   │
│  • Ledger State                                              │
└──────────────────────────────────────────────────────────────┘
```

---

## Deployment Checklist

```
┌─────────────────────────────────────────────────────────────────┐
│                    Deployment Checklist                          │
├─────────────────────────────────────────────────────────────────┤
│                                                                   │
│  Pre-Deployment                                                  │
│  ├─ [ ] Code review completed                                    │
│  ├─ [ ] All tests passing (19/19)                                │
│  ├─ [ ] Coverage ≥ 95% verified                                  │
│  ├─ [ ] Security audit completed                                 │
│  ├─ [ ] Documentation reviewed                                   │
│  └─ [ ] Integration tests passed                                 │
│                                                                   │
│  Deployment                                                      │
│  ├─ [ ] Contract compiled successfully                           │
│  ├─ [ ] WASM optimized                                           │
│  ├─ [ ] Deployed to testnet                                      │
│  ├─ [ ] Testnet validation passed                                │
│  ├─ [ ] Deployed to mainnet                                      │
│  └─ [ ] Mainnet verification completed                           │
│                                                                   │
│  Post-Deployment                                                 │
│  ├─ [ ] Monitoring enabled                                       │
│  ├─ [ ] Event tracking configured                                │
│  ├─ [ ] Frontend integration tested                              │
│  ├─ [ ] User documentation published                             │
│  └─ [ ] Support team trained                                     │
│                                                                   │
└─────────────────────────────────────────────────────────────────┘
```

---

## Performance Considerations

```
┌─────────────────────────────────────────────────────────────────┐
│                    Performance Metrics                           │
├─────────────────────────────────────────────────────────────────┤
│                                                                   │
│  Gas Usage (Estimated)                                           │
│  ├─ Authorization check:        ~1,000 gas                       │
│  ├─ Storage read:               ~5,000 gas                       │
│  ├─ Interest accrual:           ~3,000 gas                       │
│  ├─ Status validation:          ~500 gas                         │
│  ├─ Storage write:              ~5,000 gas                       │
│  ├─ Event emission:             ~2,000 gas                       │
│  └─ Total (approx):             ~16,500 gas                      │
│                                                                   │
│  Execution Time (Estimated)                                      │
│  └─ Average:                    <100ms                           │
│                                                                   │
│  Storage Impact                                                  │
│  ├─ Read operations:            1 (credit line data)             │
│  ├─ Write operations:           1 (updated credit line)          │
│  └─ Storage delta:              0 bytes (in-place update)        │
│                                                                   │
└─────────────────────────────────────────────────────────────────┘
```

---

**Document Version:** 1.0  
**Last Updated:** 2026-05-27  
**Feature Status:** ✅ Implementation Complete
