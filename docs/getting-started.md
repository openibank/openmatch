# Getting Started with OpenMatch

> A hands-on guide to building with the world's first decentralized batch auction matching engine.

---

## Table of Contents

- [Prerequisites](#prerequisites)
- [Installation](#installation)
- [Core Concepts](#core-concepts)
- [Your First Epoch: Step by Step](#your-first-epoch-step-by-step)
- [Understanding the Three Planes](#understanding-the-three-planes)
- [Working with SpendRights](#working-with-spendRights)
- [Running the Matching Engine](#running-the-matching-engine)
- [Settling Trades](#settling-trades)
- [End-to-End Example](#end-to-end-example)
- [Error Handling](#error-handling)
- [Testing](#testing)
- [Next Steps](#next-steps)

---

## Prerequisites

- **Rust 1.85+** (edition 2024) — [Install Rust](https://rustup.rs)
- **Cargo** (included with Rust)
- Basic familiarity with Rust's ownership model

## Installation

### Clone and build

```bash
git clone https://github.com/openibank/OpenMatch.git
cd OpenMatch
cargo build --workspace
cargo test --workspace   # Run all 151 tests
```

### Add as a dependency

```toml
[dependencies]
openmatch-types      = { git = "https://github.com/openibank/OpenMatch" }
openmatch-ingress    = { git = "https://github.com/openibank/OpenMatch" }
openmatch-matchcore  = { git = "https://github.com/openibank/OpenMatch" }
openmatch-settlement = { git = "https://github.com/openibank/OpenMatch" }
```

---

## Core Concepts

Before writing code, understand these five key ideas:

### 1. Epoch-Based Batch Auctions

OpenMatch does **not** continuously match orders. Instead, it collects orders into
**epochs** — discrete time windows — then matches them all at once at a single
**uniform clearing price**. This eliminates front-running and ensures fairness.

Each epoch has four phases:

```
COLLECT → SEAL → MATCH → FINALIZE
```

| Phase      | Duration  | What Happens                                      |
|------------|-----------|---------------------------------------------------|
| **COLLECT**  | 1000ms    | Orders flow in; funds are escrowed                |
| **SEAL**     | 200ms     | Buffer sealed; SHA-256 batch hash committed       |
| **MATCH**    | 500ms     | Pure deterministic matching produces trades       |
| **FINALIZE** | 2000ms    | Trades settled; SpendRights consumed; receipts    |

### 2. Three-Plane Architecture

OpenMatch separates concerns into three planes:

- **Security Envelope** (`openmatch-ingress`) — validates orders, freezes funds, mints SpendRights
- **MatchCore** (`openmatch-matchcore`) — pure deterministic computation, zero side effects
- **Finality Plane** (`openmatch-settlement`) — settles trades, consumes SpendRights, checks invariants

### 3. SpendRights

A **SpendRight** is a cryptographic pre-commitment token. When you deposit funds and
place an order, the system freezes your balance and mints a SpendRight proving
those funds are reserved. The SpendRight lifecycle:

```
ACTIVE  ──settlement──▶  SPENT     (irreversible, prevents double-spend)
   │
   └──cancel/expire──▶  RELEASED  (funds returned)
```

### 4. Deterministic Matching

Given the same `SealedBatch`, every node in the network produces the **exact same**
`TradeBundle`. This is verified via a Merkle trade root hash.

### 5. Supply Conservation

After every settlement, OpenMatch verifies a mathematical invariant:

```
∀ asset: Σ(available + frozen) == Σ(deposits) - Σ(withdrawals)
```

No tokens are ever created or destroyed during matching and settlement.

---

## Your First Epoch: Step by Step

Here's the complete flow of an order through all four epoch phases.

### Phase 1: COLLECT — Accept Orders

```rust
use openmatch_ingress::{BalanceManager, EscrowManager, RiskKernel, PendingBuffer};
use openmatch_types::*;
use rust_decimal::Decimal;

// Initialize components
let node_id = NodeId([0u8; 32]);
let mut balance_mgr = BalanceManager::new();
let mut escrow_mgr = EscrowManager::new(node_id);
let mut risk_kernel = RiskKernel::new();
let mut pending_buffer = PendingBuffer::new();
let epoch = EpochId(1);

// --- User Alice deposits USDT and places a BUY order ---
let alice = UserId::new();
balance_mgr.deposit(alice, "USDT", Decimal::new(100_000, 0));

// 1. Freeze funds and mint SpendRight
let alice_sr = escrow_mgr.mint(
    &mut balance_mgr,
    OrderId::new(),
    alice,
    "USDT",
    Decimal::new(50_000, 0),  // Lock 50,000 USDT
    epoch,
).expect("Alice has enough funds");

// 2. Create the order
let mut alice_order = Order::dummy_limit(
    OrderSide::Buy,
    Decimal::new(50_000, 0),  // Price: 50,000 USDT per BTC
    Decimal::ONE,             // Quantity: 1 BTC
);
alice_order.user_id = alice;
alice_order.sequence = 0;

// 3. Validate through risk kernel
risk_kernel.validate(&alice_order).expect("Order passes risk checks");

// 4. Push into pending buffer
pending_buffer.push(alice_order).expect("Buffer has space");

// --- User Bob deposits BTC and places a SELL order ---
let bob = UserId::new();
balance_mgr.deposit(bob, "BTC", Decimal::new(2, 0));

let bob_sr = escrow_mgr.mint(
    &mut balance_mgr,
    OrderId::new(),
    bob,
    "BTC",
    Decimal::ONE,  // Lock 1 BTC
    epoch,
).expect("Bob has enough BTC");

let mut bob_order = Order::dummy_limit(
    OrderSide::Sell,
    Decimal::new(50_000, 0),  // Price: 50,000 USDT per BTC
    Decimal::ONE,             // Quantity: 1 BTC
);
bob_order.user_id = bob;
bob_order.sequence = 1;

risk_kernel.validate(&bob_order).expect("Order passes risk checks");
pending_buffer.push(bob_order).expect("Buffer has space");
```

### Phase 2: SEAL — Seal the Batch

```rust
use openmatch_ingress::BatchSealer;

// Seal the buffer — no more orders accepted
pending_buffer.seal().expect("Buffer not already sealed");

// Extract orders and seal into a SealedBatch
let orders = pending_buffer.drain().expect("Buffer is sealed");
let batch_sealer = BatchSealer::new(node_id);
let sealed_batch = batch_sealer.seal(epoch, orders);

// The sealed batch has a SHA-256 hash committing to its contents
println!("Batch hash: {:?}", hex::encode(sealed_batch.batch_hash));
println!("Order count: {}", sealed_batch.orders.len());

// Create a digest for gossip exchange with other nodes
let digest = batch_sealer.digest(&sealed_batch);
assert_eq!(digest.batch_hash, sealed_batch.batch_hash);
```

### Phase 3: MATCH — Deterministic Matching

```rust
use openmatch_matchcore::match_sealed_batch;

// This is THE core function — pure computation, zero side effects
let trade_bundle = match_sealed_batch(&sealed_batch);

// Inspect results
println!("Trades: {}", trade_bundle.trades.len());
if let Some(price) = trade_bundle.clearing_price {
    println!("Clearing price: {} USDT", price);
}
for trade in &trade_bundle.trades {
    println!("  {} @ {} x {}", trade.market, trade.price, trade.quantity);
}

// The trade root hash can be compared across nodes
println!("Trade root: {}", hex::encode(trade_bundle.trade_root));
```

### Phase 4: FINALIZE — Settle Trades

```rust
use openmatch_settlement::Tier1Settler;

let mut settler = Tier1Settler::new(1000); // LRU cache size

// Deposit and freeze funds in the settler (mirrors ingress state)
settler.deposit(alice, "USDT", Decimal::new(50_000, 0));
settler.freeze(alice, "USDT", Decimal::new(50_000, 0)).unwrap();
settler.deposit(bob, "BTC", Decimal::ONE);
settler.freeze(bob, "BTC", Decimal::ONE).unwrap();

// Settle each trade
for trade in &trade_bundle.trades {
    settler.settle_trade(trade).expect("Settlement succeeds");
}

// Verify supply conservation
settler.verify_supply("USDT").expect("USDT supply conserved");
settler.verify_supply("BTC").expect("BTC supply conserved");

// Check final balances
let alice_btc = settler.balance(alice, "BTC");
println!("Alice BTC: {} available", alice_btc.available); // 1 BTC

let bob_usdt = settler.balance(bob, "USDT");
println!("Bob USDT: {} available", bob_usdt.available);   // 50,000 USDT
```

---

## Understanding the Three Planes

### Security Envelope (`openmatch-ingress`)

The Security Envelope is the **gatekeeper**. Every order must pass through it
before reaching MatchCore. It provides five components:

| Component         | Purpose                                      |
|-------------------|----------------------------------------------|
| `BalanceManager`  | Tracks available/frozen balances per user     |
| `EscrowManager`   | Atomically freezes funds + mints SpendRights  |
| `RiskKernel`      | Validates orders against safety limits        |
| `PendingBuffer`   | Collects validated orders during COLLECT      |
| `BatchSealer`     | Seals buffer into immutable `SealedBatch`     |

**Key guarantee**: No order enters the book without frozen funds.

```rust
use openmatch_ingress::BalanceManager;

let mut bm = BalanceManager::new();
let user = UserId::new();

// Deposit
bm.deposit(user, "USDT", Decimal::new(10_000, 0));

// Freeze (escrow for an order)
bm.freeze(user, "USDT", Decimal::new(5_000, 0)).unwrap();

// Check balance
let bal = bm.balance(user, "USDT");
assert_eq!(bal.available, Decimal::new(5_000, 0));
assert_eq!(bal.frozen, Decimal::new(5_000, 0));

// Total supply is always conserved
assert_eq!(bm.total_supply("USDT"), Decimal::new(10_000, 0));
```

### MatchCore (`openmatch-matchcore`)

MatchCore is **pure computation**. It has:
- Zero side effects (no DB writes, no network calls)
- No balance checks (that's Ingress's job)
- No risk logic (that's the RiskKernel's job)
- No plugins (extensibility goes in Ingress/Settlement)

It exposes exactly one function:

```rust
fn match_sealed_batch(batch: &SealedBatch) -> TradeBundle
```

**Determinism guarantee**: Same `SealedBatch` input → same `TradeBundle` output
on every node in the universe.

```rust
use openmatch_matchcore::{match_sealed_batch, compute_clearing_price, OrderBook};

// You can also use the order book and clearing price components independently
let mut book = OrderBook::new(MarketPair::new("BTC", "USDT"));
// ... insert orders ...
let clearing = compute_clearing_price(&book);
```

### Finality Plane (`openmatch-settlement`)

The Finality Plane executes trades and enforces financial invariants:

| Component           | Purpose                                         |
|---------------------|------------------------------------------------|
| `Tier1Settler`      | Local atomic settlement (within same node)      |
| `IdempotencyGuard`  | LRU-bounded cache prevents double-settlement    |
| `SupplyConservation`| Verifies no tokens created/destroyed            |
| `WithdrawLock`      | Blocks withdrawals during MATCH/FINALIZE phases |

```rust
use openmatch_settlement::WithdrawLock;
use openmatch_types::EpochPhase;

let mut lock = WithdrawLock::new();
lock.set_phase(EpochPhase::Match);

// Withdrawals are blocked during Match and Finalize
assert!(lock.check_withdraw().is_err());

lock.set_phase(EpochPhase::Collect);
assert!(lock.check_withdraw().is_ok());
```

---

## Working with SpendRights

SpendRights are the cryptographic primitive that prevents double-spend.

### Minting

```rust
use openmatch_ingress::{BalanceManager, EscrowManager};

let mut bm = BalanceManager::new();
let mut em = EscrowManager::new(NodeId([0u8; 32]));
let user = UserId::new();

bm.deposit(user, "USDT", Decimal::new(10_000, 0));

// Mint atomically freezes funds and creates a SpendRight
let sr_id = em.mint(
    &mut bm,
    OrderId::new(),
    user,
    "USDT",
    Decimal::new(5_000, 0),
    EpochId(1),
).unwrap();

assert!(em.is_active(&sr_id));
```

### Releasing (Cancel)

```rust
// Release unfreezes funds and marks the SpendRight as RELEASED
em.release(&mut bm, sr_id).unwrap();
assert!(!em.is_active(&sr_id));

// Funds are fully available again
let bal = bm.balance(user, "USDT");
assert_eq!(bal.available, Decimal::new(10_000, 0));
```

### Consuming (Settlement)

```rust
// During settlement, SpendRights are marked as SPENT
// This is irreversible — prevents double-spend
em.mark_spent(sr_id).unwrap();

// A SPENT SpendRight cannot be released or spent again
assert!(em.mark_spent(sr_id).is_err());   // Already SPENT
assert!(em.release(&mut bm, sr_id).is_err()); // SPENT → RELEASED is invalid
```

---

## Running the Matching Engine

### Clearing Price Computation

OpenMatch uses a **uniform clearing price** — all trades in an epoch execute at
the same price. The clearing price is the midpoint of the highest crossing bid
and lowest crossing ask.

```rust
use openmatch_matchcore::{OrderBook, compute_clearing_price};

let mut book = OrderBook::new(MarketPair::new("ETH", "USDT"));

// Add bids (buy orders) at various prices
let mut bid1 = Order::dummy_limit(OrderSide::Buy, Decimal::new(2000, 0), Decimal::ONE);
bid1.sequence = 0;
book.insert_order(bid1).unwrap();

let mut bid2 = Order::dummy_limit(OrderSide::Buy, Decimal::new(1950, 0), Decimal::ONE);
bid2.sequence = 1;
book.insert_order(bid2).unwrap();

// Add asks (sell orders)
let mut ask1 = Order::dummy_limit(OrderSide::Sell, Decimal::new(1980, 0), Decimal::ONE);
ask1.sequence = 2;
book.insert_order(ask1).unwrap();

let result = compute_clearing_price(&book);
if let Some(price) = result.clearing_price {
    println!("Clearing price: {}", price);
}
```

### Self-Trade Prevention

OpenMatch blocks wash trading at the match level. If a buy and sell order have
the same `user_id`, the match is skipped:

```rust
let user = UserId::new();
let mut buy = Order::dummy_limit(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE);
buy.user_id = user;
let mut sell = Order::dummy_limit(OrderSide::Sell, Decimal::new(100, 0), Decimal::ONE);
sell.user_id = user;

let batch = SealedBatch {
    epoch_id: EpochId(1),
    orders: vec![buy, sell],
    batch_hash: [0u8; 32],
    sealed_at: chrono::Utc::now(),
    sealer_node: NodeId([0u8; 32]),
};

let bundle = match_sealed_batch(&batch);
assert!(bundle.trades.is_empty()); // Self-trade blocked
```

### Determinism Verification

After matching, verify that your node produced the same result as other nodes
by comparing the trade root hash:

```rust
use openmatch_matchcore::{compute_trade_root, verify_trade_root};

let bundle = match_sealed_batch(&sealed_batch);

// On Node A
let root_a = compute_trade_root(&bundle.trades);

// On Node B (same sealed batch → same trades → same root)
let root_b = compute_trade_root(&bundle.trades);

assert_eq!(root_a, root_b);
assert!(verify_trade_root(&bundle.trades, &root_a));
```

---

## Settling Trades

### Local Atomic Settlement (Tier 1)

When both parties are on the same node, settlement is instant:

```rust
use openmatch_settlement::Tier1Settler;

let mut settler = Tier1Settler::new(1000);
let buyer = UserId::new();
let seller = UserId::new();

// Setup initial balances (mirrors the ingress state)
settler.deposit(buyer, "USDT", Decimal::new(50_000, 0));
settler.freeze(buyer, "USDT", Decimal::new(50_000, 0)).unwrap();
settler.deposit(seller, "BTC", Decimal::ONE);
settler.freeze(seller, "BTC", Decimal::ONE).unwrap();

// Settle the trade
settler.settle_trade(&trade).expect("Settlement succeeds");

// Verify: buyer received BTC, seller received USDT
assert_eq!(settler.balance(buyer, "BTC").available, Decimal::ONE);
assert_eq!(settler.balance(seller, "USDT").available, Decimal::new(50_000, 0));
```

### Idempotency

The same trade can never be settled twice:

```rust
settler.settle_trade(&trade).unwrap();
let err = settler.settle_trade(&trade).unwrap_err();
assert!(matches!(err, OpenmatchError::TradeAlreadySettled(_)));
```

### Supply Conservation

After settlement, verify the mathematical invariant:

```rust
// This checks: Σ(available + frozen) == Σ(deposits) - Σ(withdrawals)
settler.verify_supply("USDT").expect("Supply conserved");
settler.verify_supply("BTC").expect("Supply conserved");
```

---

## End-to-End Example

Here is a complete, runnable example that demonstrates the full epoch lifecycle
across all three planes:

```rust
use openmatch_ingress::{BalanceManager, BatchSealer, EscrowManager, PendingBuffer, RiskKernel};
use openmatch_matchcore::match_sealed_batch;
use openmatch_settlement::Tier1Settler;
use openmatch_types::*;
use rust_decimal::Decimal;

fn main() {
    let node_id = NodeId([0u8; 32]);
    let epoch = EpochId(1);

    // ==================== SECURITY ENVELOPE ====================

    let mut balance_mgr = BalanceManager::new();
    let mut escrow_mgr = EscrowManager::new(node_id);
    let mut risk_kernel = RiskKernel::new();
    let mut pending_buf = PendingBuffer::new();

    // Create users and fund them
    let alice = UserId::new();
    let bob = UserId::new();
    balance_mgr.deposit(alice, "USDT", Decimal::new(100_000, 0));
    balance_mgr.deposit(bob, "BTC", Decimal::new(10, 0));

    // Alice: BUY 1 BTC @ 50,000 USDT
    let _alice_sr = escrow_mgr.mint(
        &mut balance_mgr, OrderId::new(), alice,
        "USDT", Decimal::new(50_000, 0), epoch,
    ).unwrap();

    let mut alice_order = Order::dummy_limit(
        OrderSide::Buy, Decimal::new(50_000, 0), Decimal::ONE,
    );
    alice_order.user_id = alice;
    alice_order.sequence = 0;
    risk_kernel.validate(&alice_order).unwrap();
    pending_buf.push(alice_order).unwrap();

    // Bob: SELL 1 BTC @ 50,000 USDT
    let _bob_sr = escrow_mgr.mint(
        &mut balance_mgr, OrderId::new(), bob,
        "BTC", Decimal::ONE, epoch,
    ).unwrap();

    let mut bob_order = Order::dummy_limit(
        OrderSide::Sell, Decimal::new(50_000, 0), Decimal::ONE,
    );
    bob_order.user_id = bob;
    bob_order.sequence = 1;
    risk_kernel.validate(&bob_order).unwrap();
    pending_buf.push(bob_order).unwrap();

    // ==================== SEAL ====================

    pending_buf.seal().unwrap();
    let orders = pending_buf.drain().unwrap();
    let sealer = BatchSealer::new(node_id);
    let sealed_batch = sealer.seal(epoch, orders);

    println!("Sealed batch: {} orders, hash: {}",
        sealed_batch.orders.len(),
        hex::encode(&sealed_batch.batch_hash[..8]),
    );

    // ==================== MATCHCORE ====================

    let trade_bundle = match_sealed_batch(&sealed_batch);

    println!("Matched: {} trades", trade_bundle.trades.len());
    for trade in &trade_bundle.trades {
        println!("  {} {} @ {} = {} {}",
            trade.quantity, trade.market.base,
            trade.price,
            trade.quote_amount, trade.market.quote,
        );
    }

    // ==================== FINALITY PLANE ====================

    let mut settler = Tier1Settler::new(1000);
    settler.deposit(alice, "USDT", Decimal::new(50_000, 0));
    settler.freeze(alice, "USDT", Decimal::new(50_000, 0)).unwrap();
    settler.deposit(bob, "BTC", Decimal::ONE);
    settler.freeze(bob, "BTC", Decimal::ONE).unwrap();

    for trade in &trade_bundle.trades {
        settler.settle_trade(trade).unwrap();
    }

    // Verify invariants
    settler.verify_supply("USDT").unwrap();
    settler.verify_supply("BTC").unwrap();

    // Final balances
    println!("\nFinal Balances:");
    println!("  Alice: {} BTC, {} USDT",
        settler.balance(alice, "BTC").available,
        settler.balance(alice, "USDT").available,
    );
    println!("  Bob:   {} BTC, {} USDT",
        settler.balance(bob, "BTC").available,
        settler.balance(bob, "USDT").available,
    );
}
```

**Expected output:**
```
Sealed batch: 2 orders, hash: a3f1e8c2...
Matched: 1 trades
  1 BTC @ 50000 = 50000 USDT

Final Balances:
  Alice: 1 BTC, 0 USDT
  Bob:   0 BTC, 50000 USDT
```

---

## Error Handling

All OpenMatch errors use the `OpenmatchError` enum with `OM_ERR_` prefix codes.

```rust
use openmatch_types::OpenmatchError;

match result {
    Ok(value) => { /* success */ },
    Err(OpenmatchError::InsufficientBalance { needed, available }) => {
        eprintln!("Need {} but only {} available", needed, available);
    },
    Err(OpenmatchError::InvalidSpendRight { reason }) => {
        eprintln!("SpendRight error: {}", reason);
    },
    Err(OpenmatchError::BufferAlreadySealed) => {
        eprintln!("Cannot add orders after sealing");
    },
    Err(OpenmatchError::TradeAlreadySettled(id)) => {
        eprintln!("Trade {} was already settled (idempotency guard)", id);
    },
    Err(e) => {
        eprintln!("Error: {}", e);
    },
}
```

### Error Code Ranges

| Range | Subsystem   | Example                              |
|-------|-------------|--------------------------------------|
| 1xx   | Orders      | `OM_ERR_100` — Order not found       |
| 2xx   | Balances    | `OM_ERR_200` — Insufficient balance  |
| 3xx   | SpendRight  | `OM_ERR_300` — Invalid SpendRight    |
| 4xx   | Epoch       | `OM_ERR_400` — Wrong epoch phase     |
| 5xx   | Matching    | `OM_ERR_502` — Self-trade blocked    |
| 6xx   | Settlement  | `OM_ERR_602` — Trade already settled |
| 8xx   | Security    | `OM_ERR_801` — Supply invariant      |

---

## Testing

### Run the full test suite

```bash
cargo test --workspace                # All 151 tests
cargo test --workspace -- --nocapture # With output
```

### Run tests for a specific crate

```bash
cargo test -p openmatch-types         # 51 tests
cargo test -p openmatch-matchcore     # 40 tests
cargo test -p openmatch-ingress       # 37 tests
cargo test -p openmatch-settlement    # 23 tests
```

### Run a specific test

```bash
cargo test -p openmatch-matchcore self_trade_prevention
```

### Linting

```bash
cargo clippy --workspace              # Pedantic linting
cargo fmt --all --check               # Format check
```

---

## Next Steps

Once you're comfortable with the basics:

1. **Read the architecture docs**: `docs/00-ARCHITECTURE-DESIGN.md` and `docs/OpenMatch-HLD.md`
   for the full design rationale
2. **Explore the types**: `openmatch-types` defines every struct, enum, and error
   in the system — it's the single source of truth
3. **Study MatchCore**: `matcher.rs` is ~190 lines of pure deterministic matching logic
4. **Understand security**: Read through `risk_kernel.rs` and `escrow.rs` to see
   how the Security Envelope protects the system
5. **Look at the prompt files**: The `.prompt/` directory contains 18 system prompt
   files documenting every design decision

### Building on OpenMatch

OpenMatch is designed to be embedded. Typical integration points:

- **REST/WebSocket API**: Wrap the ingress pipeline with your API framework
- **P2P Network**: Use `SealedBatch` and `BatchDigest` for gossip-based batch agreement
- **Persistence**: Snapshot `BalanceManager` and `EscrowManager` state for crash recovery
- **Custom Risk Plugins**: Extend `RiskKernel` with domain-specific checks (plugins can
  only tighten rules, never weaken them)
- **Agent SDK**: Build algorithmic trading agents that interact with the epoch lifecycle

---

<p align="center">
  <strong>Built with conviction by the <a href="https://github.com/openibank">OpeniBank Research Team</a></strong>
  <br/>
  <em>"Fair markets through deterministic matching."</em>
</p>
