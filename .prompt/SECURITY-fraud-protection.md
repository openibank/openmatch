# OpeniMatch — Fraud Protection Architecture

## Threat Model & Defense Map

```
┌──────────────────────────────────────────────────────────────────────┐
│                    ATTACK SURFACE OVERVIEW                           │
├──────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  LAYER 1: Agent-Level Fraud (rogue AI / malicious strategy)          │
│  ├── A1: Withdraw-while-frozen (drain funds mid-trade)               │
│  ├── A2: Fake trading (appear active, siphon funds)                  │
│  ├── A3: Self-trading / wash trading (manipulate price)              │
│  ├── A4: Cross-agent balance theft                                   │
│  └── A5: Infinite loss spiral (rogue strategy burns capital)         │
│                                                                      │
│  LAYER 2: Node-Level Fraud (malicious node operator)                 │
│  ├── N1: Settlement fraud (claim settlement, keep funds)             │
│  ├── N2: Order front-running (see orders in gossip, trade ahead)     │
│  ├── N3: Selective order dropping (censor specific users)            │
│  ├── N4: Fake freeze proofs (fabricate escrow attestations)          │
│  └── N5: State divergence (run modified matching code)               │
│                                                                      │
│  LAYER 3: Protocol-Level Attacks                                     │
│  ├── P1: Double-spend (same funds backing two orders)                │
│  ├── P2: Replay attacks (reuse old freeze proofs)                    │
│  ├── P3: Front-running via latency (MEV extraction)                  │
│  └── P4: Eclipse attacks (isolate a node from peers)                 │
│                                                                      │
└──────────────────────────────────────────────────────────────────────┘
```

---

## LAYER 1: Agent-Level Fraud Protection

### A1: "Withdraw While Frozen" Attack

**Attack**: Agent deposits 100K USDT → freezes 90K for orders → withdraws 100K before orders settle.

**Defense — Already Implemented in `BalanceManager`**:

```
Balance State Machine:

  deposit(100K)     freeze(90K)         withdraw(100K)?
  ───────────►  ───────────────►   ─────────────────────► ✗ REJECTED

  available: 100K   available: 10K      available: 10K
  frozen:    0       frozen:    90K      frozen:    90K

                                        withdraw(10K)? ✓ OK
                                        withdraw(11K)? ✗ InsufficientBalance
```

**Code guarantee** (`balance_manager.rs` line 78):
```rust
if entry.available < amount {
    return Err(OpenmatchError::InsufficientBalance { ... });
}
```

Withdrawal can ONLY touch `available` balance. Frozen funds are completely isolated. There is no code path that allows withdrawing frozen funds — the `withdraw()` method only checks/decrements `available`.

**Additional defense**: `RiskLimits.min_available_reserve` ensures a minimum always stays available, even beyond the freeze/available separation.

---

### A2: "Fake Trading" Attack

**Attack**: Agent submits orders designed to never match (e.g., buys at $1 when market is $50K), creating the appearance of activity while the user's real funds are withdrawn through a different channel.

**Defense — Multi-Layer**:

```
Layer 1: Freeze-on-Submit
  Even "fake" orders require REAL funds frozen.
  Submitting Buy 1 BTC @ $1 still freezes $1 USDT.
  → Attacker's capital is locked, not free for withdrawal.

Layer 2: RiskLimits.max_total_exposure
  Caps total frozen capital. Agent can't freeze infinite amounts.

Layer 3: RiskLimits.max_epoch_loss / max_daily_loss
  If the agent IS somehow losing money, circuit breaker triggers.

Layer 4: Epoch-based matching
  All orders are public in the batch. Fake orders are visible in
  the sealed buffer hash. Auditable by any node.

Layer 5: Wash Trade Detection (future)
  If buy_user == sell_user in same batch → flag as suspicious.
  Requires cross-referencing agent_id → user_id mapping.
```

---

### A3: Self-Trading / Wash Trading

**Attack**: Agent places Buy at $100 and Sell at $100 for the same user, trades with itself to manipulate the last price or volume statistics.

**Defense**:

```rust
// In BatchMatcher — MUST be added as enforcement:
fn validate_no_self_trade(trade: &Trade) -> Result<()> {
    if trade.taker_user_id == trade.maker_user_id {
        return Err(OpenmatchError::InvalidOrder {
            reason: "Self-trading prohibited".into(),
        });
    }
    Ok(())
}
```

**Implementation**: Add a `SelfTradePolicy` enum:

```rust
enum SelfTradePolicy {
    /// Reject the entire batch (strict)
    RejectBatch,
    /// Skip the self-trade, match with next counterparty
    SkipAndContinue,
    /// Cancel the newer order (taker), keep the older (maker)
    CancelTaker,
    /// Cancel both orders
    CancelBoth,
}
```

**Current gap**: The batch matcher doesn't yet check `taker_user_id != maker_user_id`. This MUST be added in the fill loop.

---

### A4: Cross-Agent Balance Theft

**Attack**: Agent B (malicious) tries to read Agent A's balances, or submit orders using Agent A's user_id.

**Defense — AgentBinding isolation**:

```
┌───────────────────────────────────────────────────────┐
│              AGENT ISOLATION MODEL                    │
├───────────────────────────────────────────────────────┤
│                                                       │
│  AgentBinding {                                       │
│    agent_id: AgentA,                                  │
│    user_id: User123,    ← ONLY this user's funds      │
│    limits: RiskLimits,  ← SCOPED to this agent        │
│  }                                                    │
│                                                       │
│  AgentBinding {                                       │
│    agent_id: AgentB,                                  │
│    user_id: User456,    ← DIFFERENT user, isolated    │
│    limits: RiskLimits,                                │
│  }                                                    │
│                                                       │
│  INVARIANTS:                                          │
│  1. Agent can ONLY act on its bound user_id           │
│  2. Agent cannot query other users' balances          │
│  3. Agent context only contains its own data          │
│  4. One user_id can have at most ONE active agent     │
│     (prevents multiple agents racing on same funds)   │
│                                                       │
└───────────────────────────────────────────────────────┘
```

**Runtime enforcement**:
```rust
// In AgentRuntime, before executing any action:
fn authorize(agent: &AgentBinding, action: &AgentAction) -> Result<()> {
    match action {
        AgentAction::PlaceOrder { user_id, .. } => {
            if *user_id != agent.user_id {
                return Err("Agent not authorized for this user");
            }
        }
        // ... same for all actions
    }
    Ok(())
}
```

---

### A5: Infinite Loss Spiral

**Attack**: Agent with a buggy strategy enters a feedback loop, continuously placing losing trades that drain the user's balance.

**Defense — Three Circuit Breakers**:

```
┌──────────────────────────────────────────────────┐
│          LOSS CIRCUIT BREAKERS                   │
├──────────────────────────────────────────────────┤
│                                                  │
│  BREAKER 1: Per-Epoch Loss (500 USDT default)    │
│  ├── Triggers: AgentPaused                       │
│  ├── Effect: No new orders this epoch            │
│  ├── Recovery: Auto-resume next epoch            │
│  └── Rationale: Catch flash crashes              │
│                                                  │
│  BREAKER 2: Daily Loss (2K USDT default)         │
│  ├── Triggers: AgentDisabled                     │
│  ├── Effect: Agent fully stopped                 │
│  ├── Recovery: Manual admin review required      │
│  └── Rationale: Catch sustained bleeding         │
│                                                  │
│  BREAKER 3: Emergency Reserve (1K USDT default)  │
│  ├── Triggers: Order rejected BEFORE submission  │
│  ├── Effect: Can't freeze below reserve line     │
│  ├── Recovery: Deposit more funds                │
│  └── Rationale: User always has exit liquidity   │
│                                                  │
│  Loss calculation:                               │
│  epoch_pnl = Σ(received_from_settlements)        │
│            - Σ(consumed_by_settlements)          │
│  If epoch_pnl < -max_epoch_loss → PAUSE          │
│                                                  │
└──────────────────────────────────────────────────┘
```

---

## LAYER 2: Node-Level Fraud Protection

### N1: Settlement Fraud ("Claim Settlement, Keep Funds")

**Attack**: Node A matches a trade, tells Node B "I settled", but doesn't actually release the buyer's BTC.

**Defense — Three Tiers, Escalating Trust**:

```
TIER 1: Intra-Node (no trust needed)
  Both users on same node → single atomic DB transaction.
  Node can't cheat itself. Verified by balance invariant:
  Σ(all_balances) before == Σ(all_balances) after

TIER 2: Escrow-Verified (trust-minimized)
  ┌─────────────┐                   ┌──────────────┐
  │  Node A     │                   │  Node B      │
  │             │                   │              │
  │ 1. Freeze   │ ──freeze_proof──► │ 2. Verify    │
  │    50K USDT │                   │    proof     │
  │             │                   │              │
  │ 3. Match    │ ◄──batch_hash───  │ 3. Match     │
  │    (same    │                   │    (same     │
  │     result) │                   │     result)  │
  │             │                   │              │
  │ 4. Propose  │ ──settlement──►   │ 5. Verify    │
  │    settle   │    proposal       │    & confirm │
  │             │                   │              │
  │ 6. Receive  │ ◄──dual_signed──  │ 6. Release   │
  │    confirm  │    receipt        │    frozen    │
  │             │                   │              │
  │ 7. Release  │                   │              │
  │    frozen   │                   │              │
  └─────────────┘                   └──────────────┘

  FRAUD DETECTION:
  If step 5 doesn't happen within challenge_window (10 min):
  → Node A publishes FraudProof to network
  → Contains: freeze_proof + match_result + no_confirmation
  → Node B's reputation is slashed
  → Frozen funds on Node A are returned to user

  KEY INSIGHT: Funds were frozen BEFORE matching.
  Worst case: trade doesn't settle → funds return to user.
  User never LOSES money — they just don't get the trade.

TIER 3: On-Chain (fully trustless)
  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐
  │  Node A     │    │  Smart      │    │  Node B     │
  │             │    │  Contract   │    │             │
  │ deposit()  ─┼───►│  EscrowVault│◄───┼─ deposit()  │
  │             │    │             │    │             │
  │ freeze()   ─┼───►│  lock funds │◄───┼─ freeze()   │
  │             │    │             │    │             │
  │ submit     ─┼───►│ settleBatch │    │             │
  │ match proof │    │ verify sigs │    │             │
  │             │    │ atomic swap │    │             │
  │             │    │ both or     │    │             │
  │             │    │ neither     │    │             │
  └─────────────┘    └─────────────┘    └─────────────┘

  Contract enforces: BOTH sides release OR NEITHER.
  No trust in any node. Math enforces correctness.
```

---

### N2: Front-Running via Gossip

**Attack**: Node operator sees incoming orders during gossip COLLECT phase, inserts their own orders ahead to profit.

**Defense — Epoch Batch Auction + Uniform Clearing Price**:

```
WHY FRONT-RUNNING IS UNPROFITABLE IN OPENMATCH:

Traditional Exchange (continuous matching):
  Order arrives → matches instantly → price impact visible
  Front-runner: see order → place own order → profit from price impact

OpeniMatch (batch auction):
  Orders arrive → ALL collected → matched simultaneously → uniform price

  The front-runner's order goes into the SAME BATCH as the victim's.
  Both trade at the SAME clearing price.
  There is no "ahead" or "behind" — all orders are equal within a batch.

  Even if the front-runner knows all orders in the batch:
  - They can't get a BETTER price (uniform clearing)
  - They can't cause a WORSE price for others (price is market-determined)
  - They can ONLY add liquidity (which benefits everyone)

  The sequence number only affects time-priority within the same
  price level — it doesn't change the clearing price.
```

**Additional defense**: Commit-reveal scheme (future enhancement):
```
Phase 1 (Commit): Submit hash(order + nonce) during COLLECT
Phase 2 (Reveal): Submit actual order + nonce
→ Node can't see order content during collection
→ Completely eliminates information advantage
```

---

### N3: Selective Order Dropping

**Attack**: Malicious node receives an order via gossip but doesn't include it in its pending buffer, effectively censoring that user.

**Defense — Batch Hash Consensus**:

```
At COLLECT → MATCH boundary:
  All nodes broadcast their batch_hash

  Node A: batch_hash = SHA256(order1, order2, order3, order4)
  Node B: batch_hash = SHA256(order1, order2, order4)  ← dropped order3!
  Node C: batch_hash = SHA256(order1, order2, order3, order4)

  DETECTION: Node B's hash doesn't match A and C
  → Node B is flagged as divergent
  → Majority hash wins (A+C agree)
  → Node B must sync missing orders or be excluded from matching
  → Reputation penalty for repeated divergence
```

---

### N4: Fake Freeze Proofs

**Attack**: Malicious node creates a FreezeProof without actually freezing funds, getting orders matched against unfreeezeable-fund-backed positions.

**Defense — Cryptographic Verification**:

```rust
// Every freeze proof is ed25519-signed by the issuing node:
pub struct FreezeProof {
    signature: Vec<u8>,    // ed25519 over signing_payload()
    issuer_node: NodeId,   // public key of signer
    nonce: u64,            // anti-replay
    expires_at: DateTime,  // time-bound
}

// Verification on receiving node:
fn verify_freeze_proof(proof: &FreezeProof, known_nodes: &NodeRegistry) -> Result<()> {
    // 1. Check node is registered and not slashed
    let pubkey = known_nodes.get_key(&proof.issuer_node)?;

    // 2. Verify ed25519 signature
    pubkey.verify(&proof.signing_payload(), &proof.signature)?;

    // 3. Check not expired
    if proof.is_expired() { return Err(FreezeProofExpired); }

    // 4. Check nonce not reused (anti-replay)
    if seen_nonces.contains(&proof.nonce) { return Err(FreezeNonceReused); }
    seen_nonces.insert(proof.nonce);

    Ok(())
}
```

**What if the node signs a proof but doesn't actually freeze?**
→ At settlement time, the node must release frozen funds.
→ If frozen funds don't exist, settlement fails.
→ The counterparty publishes a FraudProof.
→ The lying node's stake is slashed and reputation destroyed.
→ The counterparty's funds are returned (they were frozen locally).

**Net result**: The fraudster loses their stake. The victim loses nothing (their freeze is returned).

---

### N5: State Divergence (Modified Matching Code)

**Attack**: A node runs modified matching code that gives preferential fills to the operator's orders.

**Defense — Deterministic Result Hash**:

```
All nodes match the same sealed buffer independently:

  Node A: result_hash = SHA256(trades from honest matcher)
  Node B: result_hash = SHA256(trades from modified matcher)  ← DIFFERENT!
  Node C: result_hash = SHA256(trades from honest matcher)

  A.result_hash == C.result_hash ≠ B.result_hash
  → Node B detected as divergent
  → Node B's trades are rejected by the network
  → Honest nodes' result becomes canonical
  → Node B's reputation slashed

  KEY: The determinism contract makes this verifiable.
  Same input (batch_hash) MUST produce same output (result_hash).
  Any deviation is mathematically provable fraud.
```

---

## LAYER 3: Protocol-Level Attack Protection

### P1: Double-Spend Prevention

**Attack**: User has 100K USDT, places two Buy orders each requiring 100K.

**Defense — Already enforced in BalanceManager.freeze()**:

```
Order 1: freeze(100K) → available: 0, frozen: 100K → ✓
Order 2: freeze(100K) → available: 0 < 100K → ✗ InsufficientBalance

The freeze operation is ATOMIC and SEQUENTIAL.
Two orders cannot freeze the same funds.
This is the fundamental invariant of the escrow-first model.
```

### P2: Replay Attack Prevention

**Defense — Nonce tracking in FreezeProof**:

```rust
// Each FreezeProof contains a unique nonce
pub struct FreezeProof {
    nonce: u64,       // must be unique per (node, user) pair
    expires_at: DateTime, // time-bounded validity
}

// Verification:
seen_nonces: HashSet<u64>  // per issuer_node

if seen_nonces.contains(&proof.nonce) {
    return Err(OM_ERR_303: FreezeNonceReused);
}
seen_nonces.insert(proof.nonce);
```

### P3: MEV (Miner Extractable Value) Prevention

Epoch-based batch auctions with uniform clearing price **structurally eliminate** most MEV:

| MEV Type | Continuous Exchange | OpeniMatch |
|----------|-------------------|------------|
| Front-running | Profitable (order before victim) | **Impossible** (same batch, same price) |
| Back-running | Profitable (order after victim) | **Impossible** (same batch) |
| Sandwich attack | Very profitable | **Impossible** (can't surround) |
| Time-bandit | Possible (reorg) | **N/A** (off-chain) |

### P4: Eclipse Attack Prevention

**Defense**: Multiple bootstrap nodes, Kademlia DHT, and mDNS for local discovery. Minimum peer count requirement before participating in matching.

---

## COMPLETE PROTECTION SUMMARY

```
┌──────────────────────────────────────────────────────────────────┐
│                 DEFENSE-IN-DEPTH LAYERS                          │
├──────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌─── LAYER 0: MATHEMATICAL INVARIANTS ───────────────────────┐  │
│  │ • Frozen funds cannot be withdrawn (code-enforced)         │  │
│  │ • Σ(available + frozen) is conserved across all operations │  │
│  │ • Double-freeze is impossible (atomic sequential freeze)   │  │
│  │ • Negative balances are impossible (all ops check >= 0)    │  │
│  └────────────────────────────────────────────────────────────┘  │
│                                                                  │
│  ┌─── LAYER 1: AGENT SANDBOX ─────────────────────────────────┐  │
│  │ • RiskGate validates every action before execution         │  │
│  │ • Exposure ceiling caps total frozen capital               │  │
│  │ • Loss circuit breakers (per-epoch + daily)                │  │
│  │ • Emergency reserve prevents total lockup                  │  │
│  │ • Agent isolation (can't touch other users' funds)         │  │
│  │ • No direct BalanceManager access                          │  │
│  └────────────────────────────────────────────────────────────┘  │
│                                                                  │
│  ┌─── LAYER 2: CRYPTOGRAPHIC PROOFS ──────────────────────────┐  │
│  │ • Ed25519 signed freeze proofs (unforgeable)               │  │
│  │ • Nonce-based replay prevention                            │  │
│  │ • Time-bounded proof validity (expires_at)                 │  │
│  │ • Dual-signed receipts for cross-node settlement           │  │
│  │ • Deterministic result hash for matching verification      │  │
│  └────────────────────────────────────────────────────────────┘  │
│                                                                  │
│  ┌─── LAYER 3: ECONOMIC INCENTIVES ───────────────────────────┐  │
│  │ • Reputation system (repeated fraud → exclusion)           │  │
│  │ • Staking requirement for nodes (slashable)                │  │
│  │ • Fraud proof publication (public accountability)          │  │
│  │ • Challenge windows with timeout protection                │  │
│  └────────────────────────────────────────────────────────────┘  │
│                                                                  │
│  ┌─── LAYER 4: ON-CHAIN FALLBACK ─────────────────────────────┐  │
│  │ • Smart contract escrow (fully trustless)                  │  │
│  │ • Atomic settlement (both sides or neither)                │  │
│  │ • On-chain arbitration for disputes                        │  │
│  │ • Immutable audit trail on blockchain                      │  │
│  └────────────────────────────────────────────────────────────┘  │
│                                                                  │
│  ┌─── LAYER 5: STRUCTURAL (PROTOCOL DESIGN) ──────────────────┐  │
│  │ • Escrow-FIRST: no order without frozen funds              │  │
│  │ • Batch auction: eliminates front-running                  │  │
│  │ • Uniform clearing price: eliminates latency advantage     │  │
│  │ • Deterministic matching: detects modified code            │  │
│  │ • Batch hash consensus: detects order dropping             │  │
│  └────────────────────────────────────────────────────────────┘  │
│                                                                  │
└──────────────────────────────────────────────────────────────────┘

WORST CASE FOR EACH ACTOR:

  Malicious Agent → paused/disabled, limited loss via circuit breakers
  Malicious Node  → stake slashed, reputation destroyed, trades rejected
  Both Collude    → on-chain arbitration, blockchain-enforced settlement

  IN ALL CASES: The victim's original funds are either:
  1. Successfully traded (honest settlement), or
  2. Returned to available balance (failed settlement / freeze return)

  Funds are NEVER permanently lost due to fraud.
  They are either traded or returned. No third state exists.
```

---

## GAPS TO IMPLEMENT (Prioritized)

### Critical (Must Have Before Production)

1. **Self-trade prevention in BatchMatcher** — Add `taker_user_id != maker_user_id` check in fill loop
2. **Settlement idempotency** — `HashSet<TradeId>` of already-settled trades to prevent double-settlement
3. **Withdraw-during-settle lock** — Block withdrawals during SETTLE phase for affected users
4. **Freeze proof verification** — Full ed25519 verification in order submission path

### Important (Should Have)

5. **Agent action audit log** — Append-only log of every proposed/executed/rejected action
6. **Balance invariant assertion** — Debug-mode check that `Σ(available + frozen)` is conserved after every operation
7. **Cross-node settlement timeout** — Return frozen funds if settlement doesn't confirm within window
8. **Wash trade detection** — Flag same-user trades for review

### Nice to Have

9. **Commit-reveal scheme** — Encrypted order submission to prevent gossip-based front-running
10. **Node staking + slashing** — Economic penalties for proven fraud
11. **Rate limiting per agent** — Already designed in `RiskLimits`, needs runtime implementation
