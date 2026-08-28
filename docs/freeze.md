# Freeze Contract Specification

The `Freeze` contract (`creditra-freeze`) provides account-level and global protocol emergency freeze capabilities for Creditra.

## Architecture & State Data Model

The contract relies on instance and persistent storage keys defined in `DataKey`:

- `Admin` (Instance): Address of the contract administrator.
- `GlobalFreeze` (Instance): Boolean indicating whether the global emergency freeze is active.
- `Frozen(Address)` (Persistent): Boolean flag tracking whether a specific address is frozen.
- `Freezer(Address)` (Persistent): Boolean flag tracking whether an address is granted designated freezer authority.

## Authentication Coverage Audit

Every state-changing entrypoint on `FreezeContract` strictly invokes `require_auth()` on the acting address (`admin` or `freezer`).

### State-Changing Entrypoints Audit Table

| Entrypoint | Acting Address Parameter | Authentication Call | Description |
| --- | --- | --- | --- |
| `init` | `admin: Address` | `admin.require_auth()` | Initializes admin authority and global freeze status |
| `freeze` | `admin: Address` | `admin.require_auth()` | Freezes target address (admin or authorized freezer) |
| `unfreeze` | `admin: Address` | `admin.require_auth()` | Unfreezes target address (admin or authorized freezer) |
| `set_admin` | `admin: Address` | `admin.require_auth()` | Rotates contract admin authority |
| `set_freezer` | `admin: Address` | `admin.require_auth()` | Grants or revokes freezer role for an address |
| `freeze_with_freezer` | `freezer: Address` | `freezer.require_auth()` | Freezes target address via designated freezer role |
| `unfreeze_with_freezer` | `freezer: Address` | `freezer.require_auth()` | Unfreezes target address via designated freezer role |
| `batch_freeze` | `admin: Address` | `admin.require_auth()` | Batch freezes multiple target addresses |
| `batch_unfreeze` | `admin: Address` | `admin.require_auth()` | Batch unfreezes multiple target addresses |
| `toggle_global_freeze` | `admin: Address` | `admin.require_auth()` | Toggles emergency global protocol freeze |

### View Functions (Read-Only)

- `is_frozen(env, target: Address) -> bool`: Returns `true` if global freeze is active or `target` address is frozen.
- `is_globally_frozen(env) -> bool`: Returns `true` if global emergency freeze is active.
- `get_admin(env) -> Option<Address>`: Returns current contract admin address.
- `is_freezer(env, freezer: Address) -> bool`: Returns `true` if `freezer` address has authorized freezer role.

## Error Codes

- `AlreadyInitialized` (1): Contract has already been initialized.
- `NotInitialized` (2): Contract has not been initialized.
- `Unauthorized` (3): Caller is unauthorized (failed admin or freezer role check).
- `AlreadyFrozen` (4): Target address is already frozen.
- `NotFrozen` (5): Target address is not currently frozen.
- `InvalidAddress` (6): Invalid address specified.
- `SameAdmin` (7): New admin is equal to current admin.

## Events Emitted

- `("freeze", "account")` -> `FreezeEvent { acting_address, target, timestamp }`
- `("unfreeze", "account")` -> `UnfreezeEvent { acting_address, target, timestamp }`
- `("admin", "update")` -> `AdminUpdatedEvent { old_admin, new_admin, timestamp }`
- `("freezer", "update")` -> `FreezerUpdatedEvent { admin, freezer, enabled, timestamp }`
- `("freeze", "global")` -> `GlobalFreezeEvent { admin, frozen, timestamp }`
