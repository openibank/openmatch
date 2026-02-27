# C2 — openmatch-core

> **Status**: ✅ IMPLEMENTED
> **Crate**: `crates/openmatch-core/`
> **Depends on**: `openmatch-types`

## Purpose

The core matching engine: order book, pending buffer, batch matcher, and balance manager. This is the computational heart of OpeniMatch.

## File Structure

```
crates/openmatch-core/src/
├── lib.rs              # Re-exports
├── price_level.rs      # PriceLevel: VecDeque<Order> at a single price
├── orderbook.rs        # OrderBook: BTreeMap-based, price-time priority
├── pending_buffer.rs   # PendingBuffer: collects + seals orders per epoch
├── clearing.rs         # Uniform clearing price computation
├── batch_matcher.rs    # BatchMatcher: deterministic matching algorithm
└── balance_manager.rs  # BalanceManager: per-user per-asset ledger
```

## Components

### OrderBook (`orderbook.rs`)
- Bids: `BTreeMap<Reverse<Decimal>, PriceLevel>` — highest price first
- Asks: `BTreeMap<Decimal, PriceLevel>` — lowest price first
- Index: `HashMap<OrderId, (OrderSide, Decimal)>` for O(log N) cancel
- Methods: `insert_order`, `cancel_order`, `best_bid/ask`, `spread`, `mid_price`, `drain_all`

### PendingBuffer (`pending_buffer.rs`)
- Collects orders during COLLECT phase with monotonic `sequence` counter
- `seal()` sorts deterministically: `(side, price_priority, sequence)` then computes SHA-256 batch_hash
- `take_orders()` consumes the sealed buffer for matching
- Domain-separated hash: `"openmatch:batch:v1:" || batch_id || count || order_data`

### Clearing Price (`clearing.rs`)
- Computes uniform clearing price maximizing matched volume
- For each candidate price `p`: `matchable = min(demand(p), supply(p))`
- Tie-break: smallest `|demand - supply|`, then higher price
- Returns `ClearingResult { price, volume, demand, supply }`

### BatchMatcher (`batch_matcher.rs`)
- Takes sealed PendingBuffer → produces deterministic `Vec<Trade>`
- **Algorithm**: separate buys/sells → compute clearing → walk in priority order → fill at uniform price
- **Deterministic TradeId**: `TradeId::deterministic(batch_id, fill_sequence)` — same on every node
- **Result hash**: `SHA-256("openmatch:result:v1:" || batch_id || num_trades || trade_data)`
- Returns `BatchResult { trades, result_hash, input_hash, remaining_orders, clearing_price }`

### BalanceManager (`balance_manager.rs`)
- `HashMap<(UserId, Asset), BalanceEntry>` with available/frozen tracking
- Operations: `deposit`, `withdraw`, `freeze`, `unfreeze`, `settle_trade`
- `settle_trade`: buyer's frozen quote → seller's available, seller's frozen base → buyer's available

## Determinism Contract

Given the same sealed buffer (same orders, same sequence numbers, same batch_id):
1. `input_hash` is identical across all nodes
2. `result_hash` is identical across all nodes
3. Trade IDs are identical (deterministic derivation)
4. Trade quantities and prices are identical

## Test Coverage (107 tests)

- 55 unit tests across all modules
- 5 determinism integration tests (two_matchers_same_result, repeated_matching, etc.)
- 3 full-cycle integration tests (COLLECT → MATCH → SETTLE with balance verification)
