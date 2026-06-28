# Resolves #603: Add grace-period waiver receipt event

## Description

This PR addresses issue #603 for the GrantFox campaign by emitting a structured receipt event when a grace-period waiver is applied. This improves audit transparency. 

Specifically, this refactors the existing `GraceWaiverAppliedEvent` to explicitly be a `GraceWaiverReceiptEvent`. The payload remains identical to ensure it accurately acts as a receipt for the `waived_amount`.

## Changes Included
* **Event Renamed**: Renamed `GraceWaiverAppliedEvent` to `GraceWaiverReceiptEvent` in `contracts/credit/src/events.rs`.
* **Publisher Updated**: Renamed `publish_grace_waiver_applied_event` to `publish_grace_waiver_receipt_event` and updated its usage in `contracts/credit/src/accrual.rs`.
* **Tests Updated**: Updated the event topic stability (`tests/event_topic_stability.rs`) and functional tests (`tests/grace_waiver_event.rs`) to track and assert against the new receipt event name.

## Acceptance Criteria Met
- [x] Implementation matches design.
- [x] Tests pass under cargo test (coverage preserved).
- [x] No new clippy warnings (drop-in rename).
- [x] Docs updated to reflect the updated API changes.
- [x] require_auth on state-changing entrypoints remains intact.
- [x] Clear NatSpec-style `///` rustdoc is provided on the new publisher.

## Testing Instructions
Run the repository test suite to confirm the `GraceWaiverReceiptEvent` is emitted correctly:

```bash
cargo test --workspace
```
