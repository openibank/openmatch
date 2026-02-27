# C3 — openmatch-epoch

> **Status**: 🔲 TODO
> **Crate**: `crates/openmatch-epoch/`
> **Depends on**: `openmatch-types`, `openmatch-core`

## Purpose

The epoch controller — an async state machine that orchestrates the COLLECT → MATCH → SETTLE lifecycle. It is the "heartbeat" of OpeniMatch.

## Architecture

```
EpochController (tokio task)
├── State: EpochPhase (Collect | Match | Settle)
├── Timer: tokio::time::Interval
├── PendingBuffer: owned, swapped each cycle
├── Channels:
│   ├── order_rx: mpsc::Receiver<Order>     ← API layer sends orders
│   ├── trade_tx: broadcast::Sender<Trade>  → downstream consumers
│   └── phase_tx: watch::Sender<EpochPhase> → API can query current phase
└── Signals:
    └── shutdown: CancellationToken
```

## Phase Lifecycle

```rust
loop {
    // ═══ COLLECT ═══
    phase_tx.send(EpochPhase::Collect);
    buffer = PendingBuffer::new(epoch_id.into_batch_id());
    collect_until = Instant::now() + config.collect_duration;

    tokio::select! {
        _ = sleep_until(collect_until) => {},
        _ = shutdown.cancelled() => break,
    }
    // Drain order_rx into buffer during collect
    while let Ok(order) = order_rx.try_recv() {
        buffer.push(order)?;
    }

    // ═══ MATCH ═══
    phase_tx.send(EpochPhase::Match);
    let batch_hash = buffer.seal()?;
    let result = tokio::time::timeout(
        config.match_timeout,
        tokio::task::spawn_blocking(move || matcher.match_batch(buffer))
    ).await??;

    for trade in &result.trades {
        trade_tx.send(trade.clone());
    }

    // ═══ SETTLE ═══
    phase_tx.send(EpochPhase::Settle);
    tokio::time::timeout(config.settle_timeout, async {
        for trade in &result.trades {
            balance_manager.settle_trade(trade, &market)?;
        }
        // Re-insert remaining orders into book
        orderbook.insert_batch(result.remaining_orders)?;
        Ok::<(), OpenmatchError>(())
    }).await??;

    epoch_id = epoch_id.next();
}
```

## Key Design Decisions

1. **spawn_blocking for matching**: CPU-heavy matching runs on blocking thread pool
2. **Timeout enforcement**: Both MATCH and SETTLE have hard timeouts
3. **Graceful degradation**: If MATCH times out, emit empty trades, advance epoch
4. **Channel-based interface**: Decouples epoch controller from API layer
5. **watch::Sender for phase**: Multiple readers can subscribe to phase changes

## Configuration

| Parameter | Default | Description |
|-----------|---------|-------------|
| `collect_duration` | 1000ms | How long to accept orders |
| `match_timeout` | 500ms | Hard timeout for matching |
| `settle_timeout` | 2000ms | Hard timeout for settlement |
| `seal_grace` | 50ms | Grace period for late orders |

## Error Handling

- `OM_ERR_400 WrongEpochPhase`: Order submitted during MATCH/SETTLE → queued for next epoch
- `OM_ERR_401 EpochTimeout`: Phase exceeded timeout → force-advance
- Recovery: On panic/timeout, log error, skip to next COLLECT phase

## Testing Strategy

1. **Mock timer tests**: Use `tokio::time::pause()` to control time
2. **Phase transition verification**: Assert correct phase sequence
3. **Timeout handling**: Verify forced advancement on timeout
4. **Order queuing**: Submit orders during MATCH, verify queued for next epoch
5. **Shutdown**: Send CancellationToken, verify clean exit
