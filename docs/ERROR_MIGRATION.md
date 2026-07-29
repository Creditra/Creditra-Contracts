# V1 to V2 `ContractError` encoding migration

## Summary

The `creditra-credit` CosmWasm package now exposes a client-side migration
helper for its serialized `ContractError` representation. This change does not
modify contract storage or add a state-changing entrypoint.

V1 used Serde's externally tagged enum format:

```json
"Unauthorized"
{"CreditLineNotFound": 42}
{"DrawNotFound": [7, 42]}
```

V2 uses an explicit version envelope and stable snake-case error code:

```json
{"version":2,"error":{"code":"unauthorized"}}
{"version":2,"error":{"code":"credit_line_not_found","details":42}}
{"version":2,"error":{"code":"draw_not_found","details":{"draw_id":7,"credit_line_id":42}}}
```

The V2 representation makes the encoding version discoverable before clients
decode variant-specific data. Named fields replace the ambiguous two-element
tuple used by `DrawNotFound`.

## Client migration

Use `decode_contract_error` while V1 and V2 producers coexist:

```rust
use creditra_credit::decode_contract_error;

let normalized = decode_contract_error(response_bytes)?;
match normalized.error {
    creditra_credit::ContractErrorKindV2::CreditLineNotFound(id) => {
        // Handle missing credit line.
    }
    _ => {}
}
```

To permanently rewrite stored or cached V1 response bytes, use
`migrate_v1_error_encoding`. It accepts V1 only and returns V2 JSON bytes:

```rust
use creditra_credit::migrate_v1_error_encoding;

let v2_bytes = migrate_v1_error_encoding(v1_bytes)?;
```

## Compatibility and failure behavior

- All ten currently supported V1 variants migrate without losing their data.
- `decode_contract_error` accepts both V1 and V2.
- Unknown variants, malformed JSON, truncated tuple payloads, negative
  identifiers, and unknown V2 versions return `ErrorMigrationError`.
- The helpers never panic and do not use unchecked arithmetic.
- `migrate_v1_error_encoding` intentionally rejects V2 input. Use
  `decode_contract_error` when the input version is not known.

## API-visible changes

The following public APIs are added:

- `ContractErrorEncodingV1`
- `ContractErrorEncodingV2`
- `ContractErrorKindV2`
- `ErrorMigrationError`
- `migrate_v1_error_encoding`
- `decode_contract_error`

Existing contract entrypoints and the existing runtime `ContractError` remain
unchanged.
