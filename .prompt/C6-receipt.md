# C6 — openmatch-receipt

> **Status**: 🔲 TODO
> **Crate**: `crates/openmatch-receipt/`
> **Depends on**: `openmatch-types`, `ed25519-dalek`, `sha2`

## Purpose

Cryptographic receipt system providing an auditable, tamper-proof record of every action.

## Architecture

```
ReceiptSigner
├── sign_receipt(payload, receipt_type) → Receipt
├── sign_order_accepted(order) → Receipt
├── sign_trade_executed(trade) → Receipt
└── sign_settlement_completed(trade_id, proof) → Receipt

ReceiptVerifier
├── verify(receipt, public_key) → bool
└── verify_chain(receipts) → bool  // linked list integrity

DualSigner (for cross-node)
├── propose(receipt) → HalfSignedReceipt
└── countersign(half_signed, our_key) → Receipt
```

## Receipt Lifecycle

1. **Create payload**: Serialize the event (order, trade, settlement) with bincode
2. **Hash payload**: SHA-256(payload) → `payload_hash`
3. **Sign**: Ed25519.sign(payload_hash) with node's signing key
4. **Store**: Append to receipt chain (previous receipt hash linkage)
5. **Verify**: Ed25519.verify(payload_hash, signature, issuer_public_key)

## Receipt Types

| Type | When | Payload |
|------|------|---------|
| `OrderAccepted` | Order enters pending buffer | Serialized Order |
| `OrderRejected` | Order fails validation | Order + rejection reason |
| `TradeExecuted` | Batch matching produces trade | Serialized Trade |
| `SettlementCompleted` | Trade settled | Trade + settlement proof |
| `FreezeConfirmed` | Balance frozen for order | FreezeProof |
| `UnfreezeCompleted` | Frozen balance released | Order + unfreeze reason |

## Dual-Signing Protocol (Cross-Node)

```
Node A                          Node B
  │                               │
  ├── sign(payload_hash) ────────▶│
  │   HalfSignedReceipt          │
  │                               ├── verify A's sig
  │◀──── countersign(receipt) ────┤
  │   DualSignedReceipt           │
  │                               │
  ├── verify B's sig              │
  └── store final receipt         └── store final receipt
```

## Testing

1. Sign and verify round-trip
2. Invalid signature rejection
3. Receipt chain integrity (tamper detection)
4. Dual-signing protocol with two key pairs
5. All receipt types serialize/deserialize correctly
