# C8 — openmatch-api

> **Status**: 🔲 TODO
> **Crate**: `crates/openmatch-api/`
> **Depends on**: `openmatch-types`, `openmatch-core`, `openmatch-epoch`, `openmatch-market`, `axum`, `tokio`

## Purpose

REST + WebSocket API server exposing the matching engine to clients.

## REST API (port :9001)

### Order Management
```
POST   /api/v1/orders              Submit order (validates freeze_proof)
DELETE /api/v1/orders/{id}         Cancel order
GET    /api/v1/orders/{id}         Get order status
GET    /api/v1/orders?user_id=X    List user's orders
```

### Market Data
```
GET /api/v1/orderbook/{market}     Orderbook snapshot (bids/asks with depth)
GET /api/v1/trades/{market}        Recent trades (last N or since timestamp)
GET /api/v1/ticker/{market}        24h ticker: last, high, low, vol, change%
GET /api/v1/klines/{market}        OHLCV candles (?interval=1m&limit=100)
```

### Account
```
GET  /api/v1/balance/{user_id}     User balances (all assets)
POST /api/v1/deposit               Deposit funds
POST /api/v1/withdraw              Withdraw funds
```

### System
```
GET /api/v1/epoch/status           Current phase, epoch_id, timing
GET /api/v1/health                 Node health + version
GET /api/v1/markets                List supported markets
GET /metrics                       Prometheus metrics
```

## WebSocket API (port :9002)

```
/ws/v1/orderbook/{market}   Real-time orderbook deltas
/ws/v1/trades/{market}       Real-time trade stream
/ws/v1/ticker/{market}       Real-time ticker updates
/ws/v1/user/{user_id}        User notifications (fills, cancels, receipts)
```

### Message Format (JSON)
```json
{"type": "trade", "data": {"id": "...", "price": "50000", "qty": "1.5"}}
{"type": "orderbook_delta", "data": {"bids": [...], "asks": [...]}}
{"type": "fill", "data": {"order_id": "...", "fill_qty": "0.5"}}
```

## Authentication

- Ed25519 signed requests: `X-API-Key: <pubkey_hex>`, `X-Signature: <sig_hex>`, `X-Timestamp: <unix_ms>`
- Signature over: `METHOD\nPATH\nTIMESTAMP\nBODY_HASH`
- Timestamp must be within ±30s of server time

## Error Response Format
```json
{ "error": "OM_ERR_101", "message": "Invalid order: price must be positive" }
```

## Rate Limiting

- Token bucket per IP: 100 req/s burst, 50 req/s sustained
- Per user: 10 orders/s, 5 cancels/s
- WebSocket: 100 messages/s per connection

## Implementation Notes

- Use `axum::Router` with tower middleware for auth, rate limiting, logging
- State: `Arc<AppState>` containing epoch controller handle, balance manager, orderbook
- WebSocket: `axum::extract::ws::WebSocket` with broadcast channels from epoch controller

## Testing

1. Integration tests with `axum::test::TestClient`
2. Order submission → verify in orderbook
3. WebSocket: connect, subscribe, verify trade stream
4. Auth: valid signature accepted, invalid rejected
5. Rate limiting: exceed limit, verify 429 response
