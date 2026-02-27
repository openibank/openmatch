# N1 — openmatch-market

> **Status**: 🔲 TODO
> **Crate**: `crates/openmatch-market/`
> **Depends on**: `openmatch-types`, `rust_decimal`, `chrono`

## Purpose

Market data aggregation: OHLCV candles, 24h ticker stats, and real-time price tracking.

## Components

### CandleBuilder
```rust
struct CandleBuilder {
    interval: Duration,          // 1m, 5m, 15m, 1h, 4h, 1d
    current: Option<Candle>,     // in-progress candle
    completed: VecDeque<Candle>, // ring buffer of completed candles
    max_history: usize,          // default 1000
}

struct Candle {
    market: MarketPair,
    interval: Duration,
    open_time: DateTime<Utc>,
    close_time: DateTime<Utc>,
    open: Decimal,
    high: Decimal,
    low: Decimal,
    close: Decimal,
    volume: Decimal,        // base asset volume
    quote_volume: Decimal,  // quote asset volume
    trade_count: u64,
}
```

- `on_trade(trade)`: update current candle or emit completed + start new
- Interval boundary detection: `trade.executed_at >= current.close_time` → emit + rotate
- Multiple intervals tracked simultaneously via `HashMap<Duration, CandleBuilder>`

### TickerAggregator
```rust
struct Ticker {
    market: MarketPair,
    last_price: Decimal,
    best_bid: Option<Decimal>,
    best_ask: Option<Decimal>,
    high_24h: Decimal,
    low_24h: Decimal,
    volume_24h: Decimal,
    quote_volume_24h: Decimal,
    change_24h: Decimal,       // absolute
    change_pct_24h: Decimal,   // percentage
    vwap_24h: Decimal,         // volume-weighted average price
    trade_count_24h: u64,
    updated_at: DateTime<Utc>,
}
```

- Sliding 24h window using circular buffer of hourly buckets
- VWAP = sum(price × qty) / sum(qty) over 24h

### PriceTracker
- Last trade price per market
- Real-time best bid/ask from orderbook

## Integration

- Receives trades from epoch controller via `broadcast::Receiver<Trade>`
- Publishes ticker updates via `watch::Sender<Ticker>` (polled by API/WS)
- Candles queried by API: `GET /api/v1/klines/{market}?interval=1m&limit=100`

## Testing

1. Candle from trade sequence: 3 trades in same minute → 1 candle
2. Interval boundary: trades spanning two intervals → 2 candles
3. Ticker 24h calculation: trades over 25 hours, verify rolling window
4. VWAP accuracy: known trade set → expected VWAP
5. Empty market: no trades → zero/None values
