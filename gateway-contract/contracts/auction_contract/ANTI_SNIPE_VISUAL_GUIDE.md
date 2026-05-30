# Anti-Snipe Mechanism - Visual Guide

## Timeline Diagrams

### Scenario 1: No Extension (Bid Before Window)

```
Configuration:
- end_time = 1000
- extension_window = 100
- extension_amount = 60

Timeline:
0                                                900                    1000
|------------------------------------------------|----------------------|
                                                 ^                      ^
                                          Extension Window         Original End
                                          Threshold

Bid at time 500:
0                500                            900                    1000
|----------------|-------------------------------|----------------------|
                 ^
                 Bid (no extension)

Result: end_time = 1000 (unchanged)
        extensions_count = 0
```

### Scenario 2: Single Extension (Late Bid)

```
Configuration:
- end_time = 1000
- extension_window = 100
- extension_amount = 60

Bid at time 950:
0                                                900   950              1000
|------------------------------------------------|-----|----------------|
                                                 ^     ^                ^
                                          Threshold   Bid          Original End

Extension Calculation:
proposed_end = 950 + 60 = 1010
1010 > 1000 → Extension triggered!

New Timeline:
0                                                900   950              1000  1010
|------------------------------------------------|-----|----------------|-----|
                                                       ^                      ^
                                                      Bid              New End

Result: end_time = 1010 (extended)
        extensions_count = 1
```

### Scenario 3: Multiple Extensions (Cap Enforcement)

```
Configuration:
- end_time = 1000
- extension_window = 100
- extension_amount = 60
- max_extensions = 2

Initial State:
0                                                900                    1000
|------------------------------------------------|----------------------|

Bid 1 at time 950 (Extension 1):
proposed_end = 950 + 60 = 1010
Result: end_time = 1010, extensions_count = 1

0                                                900   950              1000  1010
|------------------------------------------------|-----|----------------|-----|
                                                       ^                      ^
                                                     Bid 1              New End

Bid 2 at time 970 (Extension 2):
New threshold = 1010 - 100 = 910
970 >= 910 and 970 < 1010 → In window!
proposed_end = 970 + 60 = 1030
Result: end_time = 1030, extensions_count = 2

0                                                900   950  970         1000  1010  1030
|------------------------------------------------|-----|-----|---------|-----|-----|
                                                       ^     ^                      ^
                                                     Bid 1  Bid 2              New End

Bid 3 at time 990 (No Extension - Cap Reached):
New threshold = 1030 - 100 = 930
990 >= 930 and 990 < 1030 → In window!
BUT extensions_count (2) >= max_extensions (2) → NO EXTENSION
Result: end_time = 1030 (unchanged), extensions_count = 2

0                                                900   950  970    990  1000  1010  1030
|------------------------------------------------|-----|-----|-----|---|-----|-----|
                                                       ^     ^     ^              ^
                                                     Bid 1  Bid 2 Bid 3      Final End
                                                                   (no extension)
```

## State Machine Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│                        place_bid() Called                        │
└─────────────────────────────────────────────────────────────────┘
                              ↓
                    ┌─────────────────────┐
                    │ Validate Bid Amount │
                    └─────────────────────┘
                              ↓
                    ┌─────────────────────┐
                    │  Refund Previous    │
                    │      Bidder         │
                    └─────────────────────┘
                              ↓
              ┌───────────────────────────────────┐
              │ Check Anti-Snipe Configuration    │
              │ extension_window > 0 AND          │
              │ extension_amount > 0?             │
              └───────────────────────────────────┘
                      ↓                    ↓
                    YES                   NO
                      ↓                    ↓
        ┌─────────────────────────┐       │
        │ Calculate Threshold:    │       │
        │ threshold = end_time -  │       │
        │   extension_window      │       │
        └─────────────────────────┘       │
                      ↓                    │
        ┌─────────────────────────┐       │
        │ Is bid in window?       │       │
        │ now >= threshold AND    │       │
        │ now < end_time?         │       │
        └─────────────────────────┘       │
                ↓            ↓             │
              YES           NO             │
                ↓            │             │
    ┌───────────────────┐   │             │
    │ Check Cap:        │   │             │
    │ extensions_count  │   │             │
    │ < max_extensions? │   │             │
    └───────────────────┘   │             │
          ↓         ↓        │             │
        YES        NO        │             │
          ↓         │        │             │
    ┌─────────┐    │        │             │
    │Calculate│    │        │             │
    │proposed │    │        │             │
    │  _end   │    │        │             │
    └─────────┘    │        │             │
          ↓         │        │             │
    ┌─────────┐    │        │             │
    │proposed │    │        │             │
    │  _end > │    │        │             │
    │end_time?│    │        │             │
    └─────────┘    │        │             │
      ↓      ↓     │        │             │
    YES     NO     │        │             │
      ↓      │     │        │             │
  ┌───────┐  │     │        │             │
  │EXTEND │  │     │        │             │
  │end_   │  │     │        │             │
  │time   │  │     │        │             │
  │+count │  │     │        │             │
  └───────┘  │     │        │             │
      ↓      ↓     ↓        ↓             ↓
      └──────┴─────┴────────┴─────────────┘
                      ↓
            ┌─────────────────────┐
            │ Update Highest Bid  │
            │ and Bidder          │
            └─────────────────────┘
                      ↓
            ┌─────────────────────┐
            │   Save State and    │
            │   Bump TTL          │
            └─────────────────────┘
```

## Decision Tree

```
Is anti-snipe enabled?
├─ NO (window=0 or amount=0)
│  └─ Accept bid, no extension
│
└─ YES
   └─ Is bid in extension window?
      ├─ NO (now < threshold)
      │  └─ Accept bid, no extension
      │
      └─ YES (now >= threshold and now < end_time)
         └─ Have we reached max_extensions?
            ├─ YES (count >= max)
            │  └─ Accept bid, no extension
            │
            └─ NO (count < max)
               └─ Would proposed_end extend auction?
                  ├─ NO (proposed_end <= end_time)
                  │  └─ Accept bid, no extension
                  │
                  └─ YES (proposed_end > end_time)
                     └─ EXTEND: Update end_time and increment count
```

## Example Walkthrough

### Setup
```rust
init_auction(
    &auction_id,
    &0,        // start_time
    &1000,     // end_time
    &100,      // min_bid
    &0,        // min_increment_bps
    &100,      // extension_window
    &60,       // extension_amount
    &3,        // max_extensions
);
```

### Bid Sequence

#### Bid 1: Time 500, Amount 200
```
Check: 500 >= (1000 - 100) = 900? NO
Action: Accept bid, no extension
State: end_time=1000, count=0, highest=200
```

#### Bid 2: Time 950, Amount 300
```
Check: 950 >= 900? YES
Check: 950 < 1000? YES
Check: 0 < 3? YES
Calculate: proposed_end = 950 + 60 = 1010
Check: 1010 > 1000? YES
Action: EXTEND
State: end_time=1010, count=1, highest=300
```

#### Bid 3: Time 970, Amount 400
```
New threshold: 1010 - 100 = 910
Check: 970 >= 910? YES
Check: 970 < 1010? YES
Check: 1 < 3? YES
Calculate: proposed_end = 970 + 60 = 1030
Check: 1030 > 1010? YES
Action: EXTEND
State: end_time=1030, count=2, highest=400
```

#### Bid 4: Time 990, Amount 500
```
New threshold: 1030 - 100 = 930
Check: 990 >= 930? YES
Check: 990 < 1030? YES
Check: 2 < 3? YES
Calculate: proposed_end = 990 + 60 = 1050
Check: 1050 > 1030? YES
Action: EXTEND
State: end_time=1050, count=3, highest=500
```

#### Bid 5: Time 1010, Amount 600
```
New threshold: 1050 - 100 = 950
Check: 1010 >= 950? YES
Check: 1010 < 1050? YES
Check: 3 < 3? NO ← CAP REACHED
Action: Accept bid, no extension
State: end_time=1050, count=3, highest=600
```

## Visual Comparison: With vs Without Anti-Snipe

### Without Anti-Snipe
```
Auction Timeline:
0                                                                    1000
|---------------------------------------------------------------------|
                                                                 ^
                                                            Snipe at 999
                                                            (wins unfairly)

Problem: Last-second bid wins without giving others time to respond
```

### With Anti-Snipe
```
Auction Timeline:
0                                                900                 1000
|------------------------------------------------|-------------------|
                                                 ^                   ^
                                          Extension Window      Original End

Bid at 999:
0                                                900            999  1000  1059
|------------------------------------------------|--------------|---|-----|
                                                                ^         ^
                                                           Late Bid   Extended End

Result: Other bidders have 60 more seconds to respond
        Fair price discovery maintained
```

## Configuration Impact Visualization

### Conservative (Low-Value)
```
window=60, amount=30, max=2

0                                                940              1000
|------------------------------------------------|----------------|
                                                 ↑                ↑
                                            60s window      Original end

Max extension: 1000 + (30 × 2) = 1060 seconds
```

### Standard (Medium-Value)
```
window=120, amount=60, max=3

0                                        880                      1000
|----------------------------------------|------------------------|
                                         ↑                        ↑
                                    120s window             Original end

Max extension: 1000 + (60 × 3) = 1180 seconds
```

### Aggressive (High-Value)
```
window=300, amount=180, max=5

0                        700                                      1000
|------------------------|----------------------------------------|
                         ↑                                        ↑
                    300s window                            Original end

Max extension: 1000 + (180 × 5) = 1900 seconds
```

---

## Key Takeaways

1. **Extension Window**: Defines when bids trigger extensions
2. **Extension Amount**: How much time is added per late bid
3. **Max Extensions**: Caps total extensions to prevent infinite auctions
4. **Monotonic Time**: Auction end time only moves forward
5. **Fair Discovery**: Gives all participants equal opportunity to bid

---

**See Also**:
- `ANTI_SNIPE_IMPLEMENTATION.md` - Technical details
- `ANTI_SNIPE_QUICK_REFERENCE.md` - Configuration guide
- `src/lib.rs` - Implementation code
- `src/test.rs` - Test suite
