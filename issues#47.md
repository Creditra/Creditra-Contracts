**What each test covers and why:**

| Test | Function | Amount | Reason |
|---|---|---|---|
| `test_draw_credit_rejected_when_amount_is_zero` | `draw_credit` | `0` | Zero draws are no-ops that waste gas and could mask logic errors |
| `test_draw_credit_rejected_when_amount_is_negative` | `draw_credit` | `-1` | `i128` admits negatives; a negative draw would *decrease* utilization — a critical exploit |
| `test_repay_credit_rejected_when_amount_is_zero` | `repay_credit` | `0` | Zero repayments are meaningless and signal a caller bug |
| `test_repay_credit_rejected_when_amount_is_negative` | `repay_credit` | `-500` | A negative repayment would logically *increase* debt — must be rejected at the boundary |

**Coverage notes:**

The existing suite already covers the happy paths for `draw_credit` (`test_draw_credit_updates_utilized`, `test_close_credit_line_borrower_rejected_when_utilized_nonzero`, etc.) and the closed-state rejections for both functions. These four new tests close the remaining uncovered branches on the `amount <= 0` guard in `draw_credit` and add the equivalent guard (plus its branches) to `repay_credit`, bringing both functions to full branch coverage. With these additions the overall contract should comfortably exceed the 95% line/branch coverage threshold.