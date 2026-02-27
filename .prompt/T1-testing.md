# T1 — Testing Strategy

> **Applies to**: All crates in the OpeniMatch workspace

## Test Hierarchy

```
Unit Tests (inline #[cfg(test)])
├── Every module has inline tests
├── Test happy path + error paths
├── Use FreezeProof::dummy() via test-helpers feature
└── No external dependencies (no DB, no network)

Integration Tests (crate/tests/*.rs)
├── Full-cycle tests (COLLECT → MATCH → SETTLE)
├── Determinism regression tests
├── Cross-module interaction tests
└── May use mock services

Property-Based Tests (proptest)
├── Invariant: clearing_volume <= min(demand, supply)
├── Invariant: sum(trade.qty) <= order.remaining_qty per order
├── Invariant: result_hash identical across N runs
├── Invariant: balance total unchanged through freeze/unfreeze
└── Invariant: no negative balances after any operation

Benchmarks (criterion)
├── Batch matching: 1K, 10K, 100K orders
├── Orderbook: insert/cancel rate
├── Clearing price computation
├── PendingBuffer seal time
└── Settlement throughput
```

## Test Fixture Helpers

```rust
// Enable in Cargo.toml:
// [dev-dependencies]
// openmatch-types = { workspace = true, features = ["test-helpers"] }

// Available helpers:
FreezeProof::dummy(order_id, user_id, asset, amount)  // dummy proof (no real sig)

// Common test helpers to create in each crate's test modules:
fn dec(n: i64) -> Decimal { Decimal::new(n, 0) }
fn make_limit(side, price, qty) -> Order { ... }
fn make_market(side, qty) -> Order { ... }
fn setup_balances(users, amounts) -> BalanceManager { ... }
```

## Determinism Regression Suite

Fixed test vectors with known expected outputs:

```rust
#[test]
fn regression_batch_42() {
    // Fixed orders with known clearing price and trade output
    let orders = vec![
        buy(100, 10, seq=0),   // Buy 10 @ 100
        buy(99, 5, seq=1),     // Buy 5 @ 99
        sell(98, 8, seq=2),    // Sell 8 @ 98
        sell(100, 12, seq=3),  // Sell 12 @ 100
    ];
    let result = match_batch(BatchId(42), orders);
    assert_eq!(result.clearing_price, Some(dec(100)));
    assert_eq!(result.trades.len(), expected_trade_count);
    assert_eq!(result.result_hash, KNOWN_HASH_BYTES);
}
```

## Benchmark Targets

| Benchmark | Target | Notes |
|-----------|--------|-------|
| Match 1K orders | <1ms | Single core |
| Match 10K orders | <10ms | Single core |
| Match 100K orders | <100ms | Single core |
| Orderbook insert | >1M ops/s | Per core |
| Clearing price (1K levels) | <100μs | |
| Intra-node settlement | <1ms | Per trade |
| Seal buffer (10K orders) | <5ms | SHA-256 |

## CI Pipeline

```yaml
name: CI
on: [push, pull_request]
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - cargo fmt --check
      - cargo clippy --workspace -- -D warnings
      - cargo test --workspace
      - cargo bench --workspace --no-run  # compile benchmarks
```

## Code Coverage

- Target: >80% line coverage for openmatch-types and openmatch-core
- Tool: `cargo-llvm-cov` or `cargo-tarpaulin`
- Exclude: constants.rs, Display impls, unreachable error variants
