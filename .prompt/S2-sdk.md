# S2 — Python & TypeScript SDKs

> **Status**: 🔲 TODO
> **Directories**: `sdk/python/`, `sdk/typescript/`

## Purpose

Client SDKs for interacting with OpeniMatch nodes. Auto-generated from OpenAPI spec where possible, with handwritten WebSocket support.

## Python SDK (`sdk/python/`)

### Package: `openmatch-sdk`

```python
from openmatch import OpenmatchClient, OpenmatchWS

# REST client
client = OpenmatchClient(
    base_url="http://localhost:9001",
    signing_key=ed25519_private_key,  # for request signing
)

# Place order
order = await client.place_order(
    market="BTC/USDT", side="buy", price="50000", qty="1.0"
)

# Get orderbook
book = await client.get_orderbook("BTC/USDT", depth=20)

# Cancel
await client.cancel_order(order.id)

# Balances
balances = await client.get_balances(user_id)

# WebSocket
async with OpenmatchWS("ws://localhost:9002") as ws:
    async for trade in ws.subscribe_trades("BTC/USDT"):
        print(f"Trade: {trade.price} x {trade.qty}")
```

### Structure
```
sdk/python/
├── pyproject.toml
├── src/openmatch/
│   ├── __init__.py
│   ├── client.py        # OpenmatchClient (httpx async)
│   ├── websocket.py     # OpenmatchWS (websockets)
│   ├── auth.py          # Ed25519 request signing
│   ├── types.py         # Dataclasses matching Rust types
│   └── errors.py        # OpenmatchError mapping
└── tests/
    ├── test_client.py
    ├── test_types.py
    └── test_auth.py
```

### Dependencies
- `httpx` for async HTTP
- `websockets` for WebSocket
- `ed25519` for request signing
- `pydantic` or `dataclasses` for type definitions

## TypeScript SDK (`sdk/typescript/`)

### Package: `@openibank/openmatch`

```typescript
import { OpenmatchClient, OpenmatchWS } from '@openibank/openmatch';

const client = new OpenmatchClient({
  baseUrl: 'http://localhost:9001',
  signingKey: ed25519PrivateKey,
});

// Place order
const order = await client.placeOrder({
  market: 'BTC/USDT', side: 'buy', price: '50000', qty: '1.0'
});

// WebSocket
const ws = new OpenmatchWS('ws://localhost:9002');
ws.subscribeTrades('BTC/USDT', (trade) => {
  console.log(`Trade: ${trade.price} x ${trade.qty}`);
});
```

### Structure
```
sdk/typescript/
├── package.json
├── tsconfig.json
├── src/
│   ├── index.ts
│   ├── client.ts         # OpenmatchClient (fetch API)
│   ├── websocket.ts      # OpenmatchWS (native WebSocket)
│   ├── auth.ts           # Ed25519 signing (@noble/ed25519)
│   ├── types.ts          # TypeScript interfaces
│   └── errors.ts         # Error types
└── tests/
    ├── client.test.ts
    └── auth.test.ts
```

## OpenAPI Spec (`docs/openapi.yaml`)

- Generated from axum routes using `utoipa`
- Covers all REST endpoints
- Used to auto-generate SDK types where possible

## Testing

1. Type serialization: Rust → JSON → SDK type → verify fields
2. Auth signing: sign request → verify against known test vector
3. Integration: SDK → test server → verify round-trip
4. WebSocket: connect, subscribe, receive mock data
