Description
This is a smart-contract issue for the GrantFox campaign. This is a smart-contract issue for the GrantFox FWC26 campaign (Stellar Wave). Bump TTL on hot read paths in freeze.

Requirements and Context
Implement per the description
Add focused tests
Document any API/visible changes
Adhere to repo's lint and code style
Must be secure, tested, and documented
Should be efficient and easy to review
Suggested Execution
Fork the repo and create a branch
git checkout -b task/freeze-ttl-v7
Implement changes
contracts/freeze/src/lib.rs
Test and commit
Run the repo's standard test suite and lint
Cover edge cases; include output in the PR
Example commit message

fix: TTL bump freeze
Acceptance Criteria
 Implementation matches the description
 Tests added and passing
 Code review approved
 Docs updated
Guidelines
Minimum 95% test coverage with cargo test
require_auth on every state-changing entrypoint
Overflow-safe math; no unwrap() in production paths
Clear NatSpec-style /// rustdoc