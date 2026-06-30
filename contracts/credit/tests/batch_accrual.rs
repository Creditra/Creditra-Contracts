*** Begin Patch
*** Update File: contracts/credit/tests/batch_accrual.rs
@@
-use soroban_sdk::testutils::Ledger;
+// Ledger utilities are available via Env; explicit import removed to avoid
+// toolchain compatibility issues.
*** End Patch