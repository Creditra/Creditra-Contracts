# Contributing to Creditra Contracts

Thank you for contributing to Creditra! This document provides guidelines and conventions for contributing to the Creditra smart contract codebase.

---

## 1. Documentation & Repository Cleanliness Policy

### What Belongs at the Repository Root
The repository root is strictly reserved for:
- Core entry points: [`README.md`](../README.md) and [`WHITEPAPER.md`](../WHITEPAPER.md)
- Manifests and dependencies: `Cargo.toml`, `Cargo.lock`
- Configuration and toolchain files: `rust-toolchain.toml`, `.gitignore`, licensing files

**Never commit pull request descriptions, task checklists, or one-off implementation summaries to the repository root.**

### Where Write-ups and Documentation Belong
- **Pull Request Bodies**: PR descriptions, task checklists, scratch notes, and temporary implementation summaries belong in the **GitHub PR description** or issue comments. Do not commit them into git files.
- **Architectural & Design Documentation**: Long-lived design documents, system architectures, data flow diagrams, and protocol specifications belong in the [`docs/`](./) directory and must be indexed in [`docs/INDEX.md`](./INDEX.md).
- **Test Documentation**: Test helper conventions and snapshot procedures belong in [`docs/contributing-tests.md`](./contributing-tests.md).

---

## 2. Commit Style

We follow the [Conventional Commits](https://www.conventionalcommits.org/) specification:
- `feat:` — New user-facing or contract features
- `fix:` — Bug fixes
- `docs:` — Documentation changes
- `security:` — Security fixes, threat mitigations, and auth hardening
- `test:` — Adding or updating test suites
- `chore:` — Maintenance, dependency, or tooling updates

Commits must be atomic: one logical change per commit with descriptive messages.

---

## 3. Pull Request Workflow

1. **Branch Naming**: Create a dedicated feature branch off `main`:
   ```bash
   git checkout -b <type>/<short-description>
   # e.g., feat/collateral-liquidation or fix/auction-close-time-1307
   ```
2. **Issue Linking**: Include `Closes #<issue_number>` in the pull request description so issues close automatically on merge.
3. **Automated Checks**:
   - Ensure formatting passes: `cargo fmt --check`
   - Ensure linter passes: `cargo clippy -- -D warnings`
   - Ensure tests pass with line coverage target:
     ```bash
     cargo test
     cargo llvm-cov --workspace --all-targets --fail-under-lines 95
     ```
   - Ensure WASM build compiles within size budget (< 50 KB):
     ```bash
     cargo build --release --target wasm32-unknown-unknown -p creditra-credit
     ```
4. **Clean Git Tree**: Verify that `ls *.md` at the repository root lists only `README.md` and `WHITEPAPER.md`.

---

## 4. Coding & Architecture Standards

- **Error Handling**: No production `unwrap()` or `expect()`. Every fallible code path must return an explicit, typed `ContractError` variant documented in [`docs/contract-errors.md`](./contract-errors.md).
- **Authorization**: All state-changing entrypoints must enforce authorization using `.require_auth()`, as defined in [`docs/threat-model.md`](./threat-model.md).
- **Storage Hygiene**: Follow storage tier guidelines in [`docs/storage-layout.md`](./storage-layout.md) and ensure TTL extension on state mutation paths.
- **Module Documentation**: Every module must begin with an inner doc comment (`//!`) explaining WHAT the module does, HOW it operates, and WHY design decisions were made.
