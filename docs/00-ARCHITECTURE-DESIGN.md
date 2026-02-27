# OpenMatch — Architecture Design & Security Model

## The Three Hard Problems (and How We Solve Them)

### Problem 1: Order Placement During Matching (Race Conditions)

**The Flaw in Continuous Matching:**
Traditional exchanges (including viabtc) use continuous matching — orders arrive and match
instantly, one at a time. In a *decentralized* engine where multiple nodes share an orderbook,
this creates a fatal race condition:

- Node A receives Buy order → gossips it → matches on Node A
- Node B receives the same Buy order 50ms later → also matches on Node B
- Two matches for one order → double-spend → catastrophic failure

Even with conflict resolution (hash tiebreakers), this means:
- Rolled-back trades that users thought were filled
- Inconsistent balances across nodes
- MEV extraction opportunities during the gossip window

**OpenMatch Solution: Epoch-Based Batch Auction Matching**

Time is divided into discrete **epochs** (configurable, default 500ms). Each epoch has
three phases that NEVER overlap:

```
┌──────────────┐  ┌──────────────┐  ┌──────────────┐
│   COLLECT    │  │    MATCH     │  │   SETTLE     │
│  (accept     │  │  (orderbook  │  │  (execute    │
│   orders)    │  │   is LOCKED) │  │   trades)    │
│              │  │              │  │              │
│ Orders queue │  │ Batch match  │  │ Balances     │
│ into pending │  │ all pending  │  │ updated      │
│ buffer       │  │ at once      │  │ atomically   │
└──────────────┘  └──────────────┘  └──────────────┘
  Phase 1            Phase 2            Phase 3
  ~300ms             ~100ms             ~100ms
  │                  │                  │
  ▼                  ▼                  ▼
  Orders accepted    BOOK LOCKED        Trades settled
  Cancels accepted   No new orders      Receipts emitted
  Book readable      Deterministic      Next epoch starts
```

**Why this works:**
- During MATCH phase, the book is frozen — no new orders can change the state
- All nodes match the exact same batch → deterministic outcome → no conflicts
- Batch auctions are proven to reduce MEV (Sei blockchain uses this approach)
- Uniform clearing price per batch = fairer execution (no latency advantage)

**For decentralized mode:**
- During COLLECT, all nodes gossip orders into a shared pending set
- At epoch boundary, all nodes compute the same batch hash
- Nodes that disagree are detected via Merkle root comparison
- Matching is deterministic: same input batch → same output trades

---

### Problem 2: Settlement Fraud and Cheating

**The Trust Problem:**
In a multi-node network, settlement requires trust:
- If Node A and Node B match a cross-node trade, how does Node A know
  Node B actually credited the seller?
- What if Node B claims settlement but keeps the funds?
- What if a node fabricates receipts?

**OpenMatch Solution: Three-Tier Settlement Trust Model**

```
┌─────────────────────────────────────────────────────────┐
│                 SETTLEMENT TRUST TIERS                  │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  Tier 1: INTRA-NODE (Trustless — same database)         │
│  ├── Both parties on same node                          │
│  ├── Single atomic DB transaction                       │
│  ├── Instant settlement (<1ms)                          │
│  └── Receipt: node-signed (self-verifiable)             │
│                                                         │
│  Tier 2: ESCROW-VERIFIED (Trust-minimized)              │
│  ├── Parties on different nodes                         │
│  ├── Pre-funded escrow BEFORE matching begins           │
│  ├── Match only executes if both sides pre-escrowed     │
│  ├── Settlement: release escrow on both nodes           │
│  ├── Challenge window: 10 min for disputes              │
│  ├── Fraud proof: if escrow not released, evidence      │
│  │   published → node reputation slashed                │
│  └── Receipt: dual-signed (both nodes attest)           │
│                                                         │
│  Tier 3: ON-CHAIN ESCROW (Fully trustless)              │
│  ├── Smart contract holds both sides' funds             │
│  ├── Match result submitted to contract                 │
│  ├── Contract verifies match proof signatures           │
│  ├── Atomic release: both sides or neither              │
│  ├── No trust in any node required                      │
│  └── Receipt: on-chain transaction hash                 │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

**The Key Innovation: Escrow-First Cross-Node Settlement**

Traditional approach: Match first → settle later → hope it works
OpenMatch approach: **Escrow first → match only what's escrowed → settle atomically**

```
Cross-Node Trade Lifecycle:

1. COLLECT PHASE:
   User on Node A submits Buy order for 1 BTC at $50,000
   → Node A FREEZES $50,000 in user's balance (local escrow)
   → Order includes proof-of-freeze: hash(user_id, amount, nonce, node_sig)
   → Gossips order + freeze_proof to network

2. MATCH PHASE:
   Epoch boundary reached. All nodes have same batch.
   Matcher sees: Buy 1 BTC @$50K (Node A, freeze_proof ✓)
                 Sell 1 BTC @$49.9K (Node B, freeze_proof ✓)
   → Match! Trade at $49,900 (seller's price = buyer gets improvement)
   → Both sides have verified freeze proofs → settlement WILL succeed

3. SETTLE PHASE (Tier 2 — Escrow-Verified):
   a. Node A: debit $49,900 from frozen → credit to Node B's settlement account
   b. Node B: debit 1 BTC from frozen → credit to Node A's settlement account
   c. Both nodes sign SettlementConfirmation
   d. Exchange settlement confirmations (dual receipts)
   e. If Node B doesn't confirm within timeout:
      → Node A publishes fraud proof (freeze_proof + match_proof + no_confirmation)
      → Dispute resolution: on-chain arbitration or reputation slash

4. NETTING (periodic):
   Instead of settling every trade individually:
   → Aggregate all A↔B trades over N hours
   → Compute net position
   → Settle once (single on-chain tx or single transfer)
   → 90%+ reduction in settlement transactions
```

**Why this prevents fraud:**
- **Can't match without escrow:** Orders without freeze_proof are rejected
- **Can't steal escrowed funds:** Frozen balance is locked, only released by settlement
- **Can't fabricate receipts:** Dual-signed — both nodes must attest
- **Can't ignore settlement:** Challenge window + reputation slashing
- **Worst case fallback:** On-chain arbitration contract resolves disputes

---

### Problem 3: Does It Require On-Chain Trading?

**No. On-chain is one option, not a requirement.**

OpenMatch is designed as a **hybrid engine** with three settlement modes:

| Mode | Speed | Trust | Cost | Use Case |
|------|-------|-------|------|----------|
| **Intra-node** | <1ms | Full (same DB) | Zero | Same-exchange trades |
| **Escrow-verified** | <500ms | Minimized (crypto proofs) | Near-zero | Cross-node, trusted peers |
| **On-chain** | 2-30s | Trustless | Gas fees | Adversarial, regulatory, or self-custody |

**The architecture is settlement-agnostic.** The matching engine produces the same output
regardless of settlement mode. Settlement is pluggable — you choose per-trade or per-node-pair.

**When to use on-chain:**
- First trade with an unknown node (no trust established)
- Regulatory requirement (auditable on-chain proof)
- User explicitly requests self-custody settlement
- Dispute resolution (arbitration contract)

**When NOT needed:**
- Intra-node trades (99% of a single exchange's volume)
- Established node pairs with mutual trust + netting agreements
- Demo/testnet mode

---

## Architecture Summary

```
                    OpenMatch Architecture
                    ══════════════════════

    ┌─────────────────────────────────────────────────┐
    │              EPOCH CONTROLLER                   │
    │   Tick → COLLECT (300ms) → MATCH → SETTLE       │
    │   Clock source: deterministic (config or VDF)   │
    └────────────┬────────────────┬───────────────────┘
                 │                │
    ┌────────────▼──────┐  ┌──────▼─────────────────┐
    │  PENDING BUFFER   │  │   MATCH ENGINE         │
    │  ┌──────────────┐ │  │   (runs ONLY during    │
    │  │ Buy orders   │ │  │    MATCH phase)        │
    │  │ Sell orders  │ │  │                        │
    │  │ Cancel reqs  │ │  │   BTreeMap orderbook   │
    │  │ freeze_proofs│ │  │   Price-time priority  │
    │  └──────────────┘ │  │   Batch auction clear  │
    └───────────────────┘  └────────────┬───────────┘
                                        │
                           ┌────────────▼────────────┐
                           │   SETTLEMENT ENGINE     │
                           │                         │
                           │  ┌───────────────────┐  │
                           │  │ Tier 1: Intra-node│  │
                           │  │ (atomic DB tx)    │  │
                           │  ├───────────────────┤  │
                           │  │ Tier 2: Escrow    │  │
                           │  │ (freeze + verify) │  │
                           │  ├───────────────────┤  │
                           │  │ Tier 3: On-chain  │  │
                           │  │ (smart contract)  │  │
                           │  └───────────────────┘  │
                           └────────────┬────────────┘
                                        │
                           ┌────────────▼────────────┐
                           │   RECEIPT VAULT         │
                           │   Ed25519 signed        │
                           │   WorldLine anchored    │
                           │   Dual-signed (cross)   │
                           └─────────────────────────┘
```
