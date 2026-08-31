# Error-Code Mapping Preservation Across Contract Versions

## Summary

This document describes the error-code mapping preservation mechanism that ensures error discriminants remain stable across contract upgrades, preventing silent breaking changes that would affect deployed SDK clients and off-chain integrators.

## Problem Statement

The Creditra contract uses explicit discriminants for the `ContractError` enum (e.g., `Unauthorized = 1`, `NotAdmin = 2`, etc.). These discriminants are part of the contract ABI and are consumed by:

- Off-chain indexers that parse error codes from transaction logs
- SDK clients that map error codes to user-facing messages
- Risk dashboards that track error patterns
- Integration tests that validate error handling

If a contract upgrade accidentally reorders or renumbers error variants, it would silently break all these consumers without any detectable failure mode.

## Solution

The error-code mapping preservation system provides:

1. **Version tracking**: A persistent `ErrorMappingVersion` structure stored in instance storage
2. **Upgrade validation**: The `upgrade` entrypoint verifies error mapping compatibility
3. **Query API**: A `get_error_mapping_version` view function for off-chain clients
4. **CI guards**: Existing discriminant stability tests in `tests/error_discriminants.rs`

## Data Structures

### ErrorMappingVersion

```rust
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ErrorMappingVersion {
    /// The error-code mapping version (monotonically increasing).
    pub version: u32,
    /// The contract API version this error mapping is compatible with.
    pub contract_version: (u32, u32, u32),
    /// Ledger timestamp when this error mapping was set.
    pub set_at: u64,
}
```

### Storage Key

```rust
pub enum DataKey {
    // ... other keys
    /// Error-code mapping version for cross-contract version compatibility.
    ErrorMappingVersion,
    // ... other keys
}
```

## Constants

```rust
/// Error-code mapping version for cross-contract version compatibility.
///
/// This version tracks the stability of error discriminants across contract upgrades.
/// When error codes are added or changed, this version must be incremented with a
/// documented migration path for off-chain clients.
pub const ERROR_MAPPING_VERSION: u32 = 1;
```

## Contract Entrypoints

### get_error_mapping_version

```rust
pub fn get_error_mapping_version(env: Env) -> ErrorMappingVersion
```

Returns the current error-code mapping version. If not set, initializes it with the current contract's `ERROR_MAPPING_VERSION` and `CONTRACT_API_VERSION`.

**Behavior:**
- Read-only operation (no state mutation)
- Initializes error mapping on first call if not present
- Returns the stored version on subsequent calls

**Use cases:**
- Off-chain clients can query this to verify error code compatibility
- Indexers can detect when error mappings change
- SDKs can implement version-specific error handling

### upgrade

The `upgrade` entrypoint has been enhanced to validate error mapping compatibility:

```rust
pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
    // ... existing pause and auth checks ...

    // Preserve error-code mapping version across upgrades.
    let current_error_mapping = crate::storage::get_error_mapping_version(&env);
    if current_error_mapping.is_none() {
        // Initialize error mapping version if not set (backward compatibility)
        let error_version = crate::types::ErrorMappingVersion {
            version: ERROR_MAPPING_VERSION,
            contract_version: CONTRACT_API_VERSION,
            set_at: env.ledger().timestamp(),
        };
        crate::storage::set_error_mapping_version(&env, error_version);
    } else {
        // Verify that the error mapping version is compatible with the new contract
        let existing = current_error_mapping.unwrap();
        if existing.version != ERROR_MAPPING_VERSION {
            // Error mapping version mismatch - this is a breaking change
            env.panic_with_error(crate::types::ContractError::IncompatibleVersion);
        }
    }

    // ... perform WASM upgrade ...
}
```

**Behavior:**
- If error mapping is not set (backward compatibility with old contracts), initializes it
- If error mapping is set, verifies the version matches the new contract's version
- Rejects upgrades with `IncompatibleVersion` error if versions mismatch

**Security implications:**
- Prevents accidental breaking changes in error discriminants
- Forces explicit version bumps when error codes change
- Provides a clear signal to off-chain consumers when migrations are needed

## Storage Functions

### get_error_mapping_version

```rust
pub fn get_error_mapping_version(env: &Env) -> Option<ErrorMappingVersion>
```

Returns the stored error mapping version from instance storage, or `None` if not set.

### set_error_mapping_version

```rust
pub fn set_error_mapping_version(env: &Env, version: ErrorMappingVersion)
```

Persists the error mapping version to instance storage.

## Upgrade Workflow

### Normal Upgrade (No Error Code Changes)

1. Admin calls `upgrade(new_wasm_hash)`
2. Contract retrieves current error mapping version
3. Version matches new contract's `ERROR_MAPPING_VERSION`
4. Upgrade proceeds successfully
5. Error mapping version remains unchanged

### Breaking Upgrade (Error Code Changes)

1. Maintainer changes error discriminants (e.g., reorders variants)
2. Maintainer increments `ERROR_MAPPING_VERSION` constant
3. Admin attempts upgrade with new WASM
4. Contract detects version mismatch
5. Upgrade fails with `IncompatibleVersion` error
6. Maintainer must:
   - Document the breaking change
   - Provide migration path for off-chain clients
   - Coordinate upgrade with SDK/indexer updates
   - Optionally add a migration entrypoint to update the version

### Backward Compatibility (Old Contract)

1. Old contract (no error mapping) is upgraded
2. `upgrade` entrypoint detects no error mapping is set
3. Initializes error mapping with current version
4. Upgrade proceeds successfully
5. Error mapping is now tracked for future upgrades

## Testing

Comprehensive tests are provided in `tests/error_mapping_preservation.rs`:

### Initialization Tests
- `error_mapping_version_initializes_on_first_call`: Verifies initialization on first call
- `error_mapping_version_persists_across_calls`: Ensures persistence across multiple calls
- `error_mapping_version_matches_contract_constants`: Validates constant consistency

### Upgrade Compatibility Tests
- `upgrade_preserves_error_mapping_version`: Ensures version preservation across upgrades
- `upgrade_initializes_error_mapping_if_not_set`: Tests backward compatibility

### Version Mismatch Tests
- `upgrade_rejects_error_mapping_version_mismatch`: Validates rejection of incompatible upgrades

### Boundary and Edge Case Tests
- `error_mapping_version_handles_zero_timestamp`: Tests zero timestamp edge case
- `error_mapping_version_handles_multiple_upgrades`: Validates multiple upgrade scenario
- `error_mapping_version_concurrent_safety`: Tests concurrent query safety

### Integration Tests
- `error_mapping_preserves_existing_discriminant_tests`: Ensures compatibility with existing tests
- `error_mapping_version_query_is_read_only`: Validates read-only nature of query

## CI Guards

The existing discriminant stability tests in `tests/error_discriminants.rs` remain the primary CI guard:

- `error_discriminants_are_stable`: Asserts each discriminant value
- `no_duplicate_discriminants`: Ensures no duplicate codes
- `variant_count_is_known`: Validates total variant count
- `category_discriminants_are_stable`: Asserts category discriminants
- `category_mappings_are_stable`: Validates error-to-category mappings

The error mapping preservation system adds an additional layer of protection at runtime during upgrades.

## Migration Guide for Maintainers

### Adding New Error Variants

When adding a new error variant:

1. Append the variant to the end of `ContractError` enum with the next available integer
2. Add the corresponding assertion in `tests/error_discriminants.rs`
3. **Do not** increment `ERROR_MAPPING_VERSION` (adding variants is backward compatible)
4. Update documentation in `docs/ERROR_CODES.md`

### Changing Existing Error Variants

When changing an existing error variant (breaking change):

1. Increment `ERROR_MAPPING_VERSION` constant
2. Document the breaking change in a PR description
3. Update `tests/error_discriminants.rs` assertions
4. Update documentation in `docs/ERROR_CODES.md`
5. Coordinate with SDK/indexer teams for migration
6. Consider adding a migration entrypoint if needed

### Removing Error Variants

When removing an error variant (breaking change):

1. Comment out the variant in the enum (preserve the discriminant as reserved)
2. Comment out the corresponding assertion in `tests/error_discriminants.rs`
3. Increment `ERROR_MAPPING_VERSION` constant
4. Document the removal and reserved discriminant
5. Follow the same coordination process as changing variants

## Client-Side Integration

### SDK Integration

SDKs should:

1. Query `get_error_mapping_version` on initialization
2. Compare against supported error mapping versions
3. Implement version-specific error handling if needed
4. Warn users if error mapping version is unsupported

### Indexer Integration

Indexers should:

1. Track error mapping version changes via upgrade events
2. Maintain version-specific error code mappings
3. Alert operators when error mapping version changes
4. Support backward-compatible error code interpretation

### Dashboard Integration

Dashboards should:

1. Display current error mapping version
2. Highlight when error mappings change
3. Provide migration guidance to operators
4. Support version-specific error aggregation

## Security Considerations

### Authorization

- `get_error_mapping_version` is a public view function (no auth required)
- `upgrade` requires admin authentication (existing guard)
- Error mapping storage is in instance storage (admin-controlled)

### State Invariants

- Error mapping version is monotonically increasing (never decreases)
- Error mapping version is set atomically with contract version
- Upgrade rejection prevents inconsistent state

### Replay Safety

- Error mapping version is stored in instance storage (not per-transaction)
- No replay attack vectors (read-only query, upgrade is admin-gated)

## Future Enhancements

Potential future improvements:

1. **Migration entrypoint**: Add an admin entrypoint to explicitly bump error mapping version
2. **Version compatibility matrix**: Support multiple compatible error mapping versions
3. **Graceful degradation**: Allow SDKs to handle unknown error codes gracefully
4. **Error mapping history**: Track historical error mapping versions for audit trails

## References

- `contracts/credit/src/types.rs`: ErrorMappingVersion type definition
- `contracts/credit/src/storage.rs`: Storage functions for error mapping
- `contracts/credit/src/lib.rs`: get_error_mapping_version and upgrade logic
- `contracts/credit/tests/error_mapping_preservation.rs`: Comprehensive test suite
- `contracts/credit/tests/error_discriminants.rs`: Discriminant stability tests
- `docs/ERROR_CODES.md`: Error code reference documentation
