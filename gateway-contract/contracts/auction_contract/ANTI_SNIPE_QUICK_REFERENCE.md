# Anti-Snipe Mechanism - Quick Reference

## What It Does
Prevents last-second bid sniping by automatically extending auction end time when bids arrive in the final seconds.

## Configuration

### Parameters
| Parameter | Type | Description |
|-----------|------|-------------|
| `extension_window` | `u64` | Final seconds before end_time where bids trigger extensions |
| `extension_amount` | `u64` | Seconds added to end_time per late bid |
| `max_extensions` | `u32` | Maximum number of extensions allowed |
| `extensions_count` | `u32` | Current count (auto-managed, init to 0) |

### Enable/Disable
- **Enable**: Set `extension_window > 0` AND `extension_amount > 0`
- **Disable**: Set `extension_window = 0` OR `extension_amount = 0`

## How It Works

### Extension Trigger
A bid triggers an extension when:
1. Bid arrives at time `now >= end_time - extension_window`
2. AND `now < end_time` (auction still open)
3. AND `extensions_count < max_extensions` (cap not reached)
4. AND `now + extension_amount > end_time` (would actually extend)

### Extension Calculation
```
new_end_time = now + extension_amount
extensions_count += 1
```

## Examples

### Example 1: Basic Anti-Snipe
```rust
// 2-minute window, 1-minute extension, max 3 times
init_auction(
    &auction_id,
    &0,           // start_time
    &3600,        // end_time (1 hour)
    &100,         // min_bid
    &0,           // min_increment_bps
    &120,         // extension_window (2 minutes)
    &60,          // extension_amount (1 minute)
    &3,           // max_extensions
);
```

**Scenario**:
- Auction ends at 3600
- Extension window: 3480-3600 (last 2 minutes)
- Bid at 3500: Extends to 3560 (3500 + 60)
- Bid at 3550: Extends to 3610 (3550 + 60)
- Bid at 3600: Extends to 3660 (3600 + 60)
- Bid at 3650: No extension (max 3 reached)

### Example 2: Aggressive Anti-Snipe
```rust
// 5-minute window, 5-minute extension, max 5 times
init_auction(
    &auction_id,
    &0,           // start_time
    &3600,        // end_time
    &100,         // min_bid
    &0,           // min_increment_bps
    &300,         // extension_window (5 minutes)
    &300,         // extension_amount (5 minutes)
    &5,           // max_extensions
);
```

### Example 3: Disabled
```rust
// Anti-snipe disabled
init_auction(
    &auction_id,
    &0,           // start_time
    &3600,        // end_time
    &100,         // min_bid
    &0,           // min_increment_bps
    &0,           // extension_window (disabled)
    &0,           // extension_amount
    &0,           // max_extensions
);
```

## Timeline Visualization

```
Original auction: [--------------------] end=1000
                                    ^
                                    900 (extension_window=100)

Bid at 950 (within window):
New auction:      [--------------------====] end=1010
                                        ^
                                        950+60

Bid at 1000 (within new window):
Final auction:    [--------------------========] end=1060
                                            ^
                                            1000+60
```

## State Tracking

### AuctionConfig Fields
```rust
pub struct AuctionConfig {
    // ... other fields ...
    pub extension_window: u64,      // Set at init, never changes
    pub extension_amount: u64,      // Set at init, never changes
    pub max_extensions: u32,        // Set at init, never changes
    pub extensions_count: u32,      // Starts at 0, increments per extension
}
```

### Reading State
```rust
let state: AuctionState = env.storage().persistent().get(&auction_id).unwrap();
let current_end = state.config.end_time;
let extensions_used = state.config.extensions_count;
let extensions_remaining = state.config.max_extensions - extensions_used;
```

## Edge Cases Handled

✅ **Bid before window**: No extension  
✅ **Bid at exact threshold**: Extension triggered  
✅ **Bid after end_time**: Rejected (auction closed)  
✅ **Max extensions reached**: Bid accepted, no extension  
✅ **Proposed end ≤ current end**: No extension (monotonic check)  
✅ **Overflow protection**: All arithmetic uses `checked_add()`/`checked_sub()`  
✅ **Window = 0**: Anti-snipe disabled  
✅ **Amount = 0**: Anti-snipe disabled

## Testing

### Run Tests
```bash
cargo test -p auction_contract snipe
```

### Test Coverage
- Pre-window bids (no extension)
- Late bids (extension triggered)
- Extension cap enforcement
- Disabled via window=0
- Disabled via amount=0
- Exact threshold behavior
- Monotonic end_time check

## Security Considerations

1. **Bounded Duration**: `max_extensions` prevents infinite auctions
2. **Overflow Safety**: All time calculations use checked arithmetic
3. **Monotonic Time**: Extensions only increase end_time
4. **Deterministic**: Same inputs always produce same results
5. **No Griefing**: Extensions don't prevent legitimate bids

## Migration Guide

### Updating Existing Code

**Before**:
```rust
client.init_auction(&id, &start, &end, &min_bid, &bps);
```

**After** (disabled):
```rust
client.init_auction(&id, &start, &end, &min_bid, &bps, &0, &0, &0);
```

**After** (enabled):
```rust
client.init_auction(&id, &start, &end, &min_bid, &bps, &120, &60, &3);
```

## Recommended Settings

### Conservative (Low-Value Auctions)
- `extension_window`: 60 seconds
- `extension_amount`: 30 seconds
- `max_extensions`: 2

### Standard (Medium-Value Auctions)
- `extension_window`: 120 seconds
- `extension_amount`: 60 seconds
- `max_extensions`: 3

### Aggressive (High-Value Auctions)
- `extension_window`: 300 seconds
- `extension_amount`: 180 seconds
- `max_extensions`: 5

### Disabled (Testing/Legacy)
- `extension_window`: 0
- `extension_amount`: 0
- `max_extensions`: 0

---

**See Also**:
- `ANTI_SNIPE_IMPLEMENTATION.md` - Full technical documentation
- `src/lib.rs` - Implementation code
- `src/test.rs` - Test suite
