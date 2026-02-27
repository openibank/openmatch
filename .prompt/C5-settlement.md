# C5 — openmatch-settlement

> **Status**: 🔲 TODO
> **Crate**: `crates/openmatch-settlement/`
> **Depends on**: `openmatch-types`, `openmatch-core`, `openmatch-escrow`, `openmatch-receipt`

## Purpose

Three-tier settlement engine that resolves trades based on trust level between counterparties.

## Architecture

```
SettlementRouter
├── resolve_tier(trade) → SettlementTier
├── IntraNodeSettler    (Tier 1: same node, <1ms)
├── EscrowVerifiedSettler (Tier 2: cross-node, ~500ms)
└── OnChainSettler      (Tier 3: smart contract, 2-30s)
```

## Tier 1: Intra-Node (Same Database)

- Both parties on the same node
- Calls `BalanceManager::settle_trade()` directly — single atomic operation
- Issues node-signed `Receipt` with type `SettlementCompleted`
- Latency: <1ms

## Tier 2: Escrow-Verified (Cross-Node)

```
State Machine: Pending → Proposed → Confirmed → Completed
                                  → Challenged → Resolved

1. Both nodes have verified freeze_proofs (from COLLECT phase)
2. Matcher node proposes settlement with signed SettlementProposal
3. Counterparty node verifies proposal + releases frozen funds
4. Both nodes exchange dual-signed SettlementConfirmation
5. If no confirmation within challenge_window (default 10min):
   → Proposer publishes FraudProof { freeze_proof, match_proof, no_confirmation }
   → Reputation slash on non-responding node
```

### Key Types
```rust
struct SettlementProposal {
    trade: Trade,
    proposer_node: NodeId,
    proposer_signature: Vec<u8>,
    proposed_at: DateTime<Utc>,
}

struct SettlementConfirmation {
    trade_id: TradeId,
    proposer_sig: Vec<u8>,
    confirmer_sig: Vec<u8>,
    confirmed_at: DateTime<Utc>,
}

struct FraudProof {
    trade: Trade,
    freeze_proof: FreezeProof,
    match_proof: Vec<u8>,
    challenge_deadline: DateTime<Utc>,
}
```

## Tier 3: On-Chain (Smart Contract)

- Builds EVM transaction data for `EscrowVault.settleBatch()`
- Submits via ethers-rs (or mock in tests)
- Waits for on-chain confirmation
- Issues receipt with on-chain tx hash

## Netting

Periodic aggregation of cross-node trades:
1. Accumulate all trades between Node A ↔ Node B over N hours
2. Compute net position per asset
3. Single settlement transaction for the net amount
4. 90%+ reduction in settlement operations

## Agent Asset Protection

- **Hard freeze enforcement**: Funds frozen for an order cannot be released except by:
  1. Trade settlement (frozen → counterparty)
  2. Order cancellation (frozen → available, only if order not yet matched)
  3. Freeze expiry (frozen → available, after expires_at)
- **No double-settlement**: Settlement is idempotent — trade_id checked against settled set
- **Timeout protection**: Challenge windows prevent indefinite fund lockup
- **Agent risk limits**: Per-agent maximum frozen amount, enforced at order submission

## Testing

1. Intra-node: settle trade, verify balances
2. Cross-node mock: simulate two-node settlement with channels
3. Timeout: verify challenge window triggers fraud proof
4. Netting: aggregate trades, verify net position calculation
5. Idempotency: settle same trade twice, verify no double-execution
