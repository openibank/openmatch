# C7 — openmatch-gossip

> **Status**: 🔲 TODO
> **Crate**: `crates/openmatch-gossip/`
> **Depends on**: `openmatch-types`, `libp2p`, `tokio`

## Purpose

P2P gossip network for order propagation, batch consensus, and result verification across nodes.

## Architecture

```
GossipNode
├── Transport: TCP + Noise + Yamux (libp2p)
├── Protocols:
│   ├── GossipSub  → order/batch/result broadcast
│   ├── Kademlia   → peer discovery (DHT)
│   └── mDNS       → local peer discovery
├── Topics:
│   ├── orders/{market}   — new orders + freeze_proofs (COLLECT)
│   ├── batches/{market}  — sealed buffer hashes (MATCH boundary)
│   ├── results/{market}  — trade result hashes (MATCH end)
│   └── control           — peer status, epoch sync
└── Dedup: LRU cache of seen message hashes
```

## Message Types

```rust
enum GossipMessage {
    /// New order with freeze proof (during COLLECT)
    OrderGossip { order: Order, freeze_proof: FreezeProof },
    /// Sealed buffer hash for consensus (COLLECT → MATCH boundary)
    BatchSealed { batch_id: BatchId, batch_hash: [u8; 32], order_count: u64 },
    /// Match result hash for verification (MATCH → SETTLE boundary)
    TradeResult { batch_id: BatchId, result_hash: [u8; 32], trade_count: u64 },
    /// Node status heartbeat
    PeerStatus { node_id: NodeId, epoch_id: EpochId, phase: EpochPhase },
}
```

## Phase-Specific Behavior

| Phase | Gossip Activity |
|-------|----------------|
| COLLECT | Broadcast new orders; receive orders from peers; add to PendingBuffer |
| MATCH | Broadcast batch_hash; verify peers agree; flag disagreements |
| SETTLE | Broadcast result_hash; verify peers computed same trades |

## Consensus Verification

At each phase boundary, nodes compare hashes:
- **batch_hash mismatch**: Node has different order set → log warning, request missing orders
- **result_hash mismatch**: Determinism violation → log CRITICAL, halt matching for this batch

## Rate Limiting & Security

- Per-peer message rate limit (token bucket)
- Message deduplication via SHA-256 hash → LRU seen cache (10K entries)
- Message size limit: 1MB max per gossip message
- Peer reputation scoring: drop peers with high mismatch rate

## Testing

1. Mock transport: in-memory libp2p channels
2. Order gossip round-trip: send from node A, receive on node B
3. Batch hash consensus: 3 nodes agree on same hash
4. Deduplication: same message twice, only processed once
5. Rate limiting: exceed rate, verify messages dropped
