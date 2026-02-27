# C9 — openmatch-persistence

> **Status**: 🔲 TODO
> **Crate**: `crates/openmatch-persistence/`
> **Depends on**: `openmatch-types`, `openmatch-core`, `sqlx`, `tokio`

## Purpose

PostgreSQL persistence + AOF crash recovery for durable state management.

## PostgreSQL Schema

```sql
-- Orders
CREATE TABLE orders (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL,
    market VARCHAR(20) NOT NULL,  -- "BTC/USDT"
    side VARCHAR(4) NOT NULL,     -- "BUY" / "SELL"
    order_type VARCHAR(10) NOT NULL,
    status VARCHAR(20) NOT NULL,
    price NUMERIC(28,8),
    quantity NUMERIC(28,8) NOT NULL,
    remaining_qty NUMERIC(28,8) NOT NULL,
    batch_id BIGINT,
    sequence BIGINT NOT NULL DEFAULT 0,
    origin_node BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX idx_orders_market_status ON orders(market, status);
CREATE INDEX idx_orders_user ON orders(user_id, market);

-- Trades
CREATE TABLE trades (
    id UUID PRIMARY KEY,
    batch_id BIGINT NOT NULL,
    market VARCHAR(20) NOT NULL,
    taker_order_id UUID NOT NULL REFERENCES orders(id),
    maker_order_id UUID NOT NULL REFERENCES orders(id),
    taker_user_id UUID NOT NULL,
    maker_user_id UUID NOT NULL,
    price NUMERIC(28,8) NOT NULL,
    quantity NUMERIC(28,8) NOT NULL,
    quote_amount NUMERIC(28,8) NOT NULL,
    taker_side VARCHAR(4) NOT NULL,
    matcher_node BYTEA NOT NULL,
    executed_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX idx_trades_market ON trades(market, executed_at DESC);
CREATE INDEX idx_trades_batch ON trades(batch_id);

-- Balances
CREATE TABLE balances (
    user_id UUID NOT NULL,
    asset VARCHAR(10) NOT NULL,
    available NUMERIC(28,8) NOT NULL DEFAULT 0,
    frozen NUMERIC(28,8) NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, asset)
);

-- Epochs
CREATE TABLE epochs (
    epoch_id BIGINT PRIMARY KEY,
    phase VARCHAR(10) NOT NULL,
    batch_hash BYTEA,
    result_hash BYTEA,
    trade_count INT DEFAULT 0,
    started_at TIMESTAMPTZ NOT NULL,
    completed_at TIMESTAMPTZ
);
```

## AOF (Append-Only File) for Crash Recovery

```
Format per entry: [length:u32][checksum:u32][operation:bincode]

Operations:
- OrderReceived(Order)
- OrderCancelled(OrderId)
- BufferSealed(BatchId, batch_hash)
- TradeExecuted(Trade)
- BalanceUpdated(UserId, Asset, available, frozen)
- EpochAdvanced(EpochId, EpochPhase)
```

### Write-Ahead Pattern
1. Write operation to AOF (fsync)
2. Apply to in-memory state
3. Async flush to PostgreSQL (batch every 100ms)

### Recovery on Startup
1. Load latest snapshot from PostgreSQL
2. Replay AOF from last snapshot's epoch_id
3. Verify state consistency
4. Resume from recovered state

### Compaction
- Periodic: snapshot full state to PostgreSQL, truncate AOF
- Trigger: AOF size > 100MB or age > 1 hour

## Connection Pool

- `sqlx::PgPool` with configurable max_connections (default 10)
- Migrations via `sqlx::migrate!`
- All queries use `rust_decimal::Decimal` for NUMERIC columns

## Testing

1. In-memory SQLite for fast unit tests (`sqlx::SqlitePool`)
2. AOF: write operations, replay, verify state matches
3. Snapshot: dump state, reload, verify equality
4. Crash simulation: write partial AOF, verify recovery
5. Connection pool: concurrent reads/writes under load
