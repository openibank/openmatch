# C1 — openmatch-types

> **Status**: ✅ IMPLEMENTED
> **Crate**: `crates/openmatch-types/`
> **Depends on**: (none — leaf crate)

## Purpose

The shared type library for the entire OpeniMatch workspace. Every other crate depends on this. It defines all domain types, error codes, configuration, and constants.

## File Structure

```
crates/openmatch-types/src/
├── lib.rs          # Re-exports all modules
├── ids.rs          # OrderId, UserId, NodeId, TradeId, BatchId, MarketPair
├── order.rs        # OrderSide, OrderType, OrderStatus, Order
├── trade.rs        # Trade
├── freeze.rs       # FreezeProof
├── receipt.rs      # Receipt, ReceiptType
├── epoch.rs        # EpochPhase, EpochId, EpochConfig
├── balance.rs      # BalanceEntry, Asset
├── config.rs       # NodeConfig, NetworkConfig, MarketConfig
├── error.rs        # OpenmatchError (OM_ERR_ prefix)
└── constants.rs    # System-wide limits and defaults
```

## Key Types

### Identifiers (`ids.rs`)
- **OrderId(Uuid)** — UUIDv7, time-ordered, `Ord + Hash + Serialize`
- **UserId(Uuid)** — UUIDv7
- **NodeId([u8; 32])** — ed25519 public key, `Display` shows hex prefix
- **TradeId(Uuid)** — has `deterministic(batch_id, fill_seq)` constructor for cross-node consistency
- **BatchId(u64)** — monotonically increasing, has `next()` method
- **MarketPair { base, quote }** — canonical format `"BTC/USDT"`

### Order Model (`order.rs`)
- **OrderSide**: `Buy | Sell` — derives `Ord` so `Buy < Sell` for deterministic sorting
- **OrderType**: `Limit | Market | Cancel`
- **OrderStatus**: `PendingFreeze | Active | PartiallyFilled | Filled | Cancelled | Rejected | Expired`
- **Order**: full struct with `freeze_proof: FreezeProof`, `sequence: u64`, `batch_id: Option<BatchId>`
  - `effective_price()` → Limit uses stated price, Market Buy = MAX, Market Sell = ZERO
  - `is_matchable_at(price)`, `is_filled()`, `fill_ratio()`

### FreezeProof (`freeze.rs`)
- Ed25519-signed attestation: `order_id || user_id || asset || amount || nonce`
- `is_expired()`, `signing_payload()` for deterministic verification
- `FreezeProof::dummy()` available under `test-helpers` feature

### Error Codes (`error.rs`)
All errors use `OM_ERR_` prefix:
- 1xx: Order (NotFound, Invalid, Duplicate, NotCancellable, LimitExceeded)
- 2xx: Balance (InsufficientBalance, InsufficientFrozen, Underflow)
- 3xx: Freeze (InvalidProof, Expired, SignatureInvalid, NonceReused)
- 4xx: Epoch (WrongPhase, Timeout, BufferSealed, BufferFull)
- 5xx: Matching (Failed, DeterminismViolation)
- 6xx: Settlement (Failed, OnChainRejected)
- 7xx: Network (NodeNotFound, GossipError, PeerConnectionFailed)
- 9xx: General (Internal, Serialization, Configuration, Io)

## Design Rules

1. All financial quantities use `rust_decimal::Decimal` — **never** floating point
2. All types derive `Serialize, Deserialize` for JSON and bincode
3. Types used as BTreeMap keys must derive `Ord, PartialOrd`
4. UUIDv7 for all entity IDs (time-ordered sorting)
5. `test-helpers` feature gates `FreezeProof::dummy()` for unit tests
6. `Result<T>` alias = `std::result::Result<T, OpenmatchError>`
