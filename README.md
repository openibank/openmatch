<p align="center">
  <br/>
  <strong>O P E N M A T C H</strong>
  <br/>
  <em>The World's First Decentralized Batch Auction Matching Engine</em>
  <br/>
  <br/>
  <a href="#architecture"><img src="https://img.shields.io/badge/architecture-three--plane-blueviolet?style=flat-square" alt="Three-Plane Architecture"/></a>
  <a href="#security-model"><img src="https://img.shields.io/badge/security-bank--grade-green?style=flat-square" alt="Bank-Grade Security"/></a>
  <a href="#design-principles"><img src="https://img.shields.io/badge/determinism-100%25-blue?style=flat-square" alt="100% Deterministic"/></a>
  <a href="https://github.com/openibank/OpenMatch/actions"><img src="https://img.shields.io/badge/tests-165%20passing-brightgreen?style=flat-square" alt="Tests"/></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT%2FApache--2.0-orange?style=flat-square" alt="License"/></a>
</p>

---

```
    +==================================================================+
    |                                                                  |
    |   "What if a matching engine had zero side effects,              |
    |    provable determinism, and cryptographic settlement --         |
    |    and you could run it on a thousand nodes?"                    |
    |                                                                  |
    +==================================================================+
```

**OpenMatch** is an epoch-based, deterministic batch auction matching engine
designed for decentralized finance infrastructure. It solves the three hardest
problems in distributed exchange design -- **race conditions**, **double-spend**,
and **cross-node determinism** -- through a novel three-plane architecture with
cryptographic pre-commitments called **SpendRights**.

Built in Rust. Zero `unsafe`. 165 tests. Production-grade.

> Copyright (c) 2025 [OpeniBank Research Team](https://github.com/openibank).
> Licensed under MIT or Apache-2.0.

---

## Table of Contents

- [Why OpenMatch Exists](#why-openmatch-exists)
- [Architecture](#architecture)
- [Security Model](#security-model)
- [Crate Map](#crate-map)
- [Quick Start](#quick-start)
- [Design Principles](#design-principles)
- [Error Codes](#error-codes)
- [Roadmap](#roadmap)
- [Contributing](#contributing)
- [License](#license)

---

## Why OpenMatch Exists

Traditional matching engines were built for centralized exchanges -- single server,
single order book, single point of failure. When you try to distribute them across
nodes, three catastrophic problems emerge:

| Problem | What Happens | How Everyone Else Fails |
|---------|-------------|------------------------|
| **Race Conditions** | Two nodes match the same order simultaneously | Rolled-back trades, broken trust |
| **Double-Spend** | Frozen funds are consumed twice during settlement | Phantom balances, insolvency |
| **Non-Determinism** | Different nodes produce different trades from the same input | Consensus failure, chain forks |

OpenMatch was built from first principles to make these problems **structurally impossible**.

---

## Architecture

```
+---------------------------------------------------------------------+
|                     EPOCH LIFECYCLE (4 Phases)                      |
|                                                                     |
|  +----------+   +--------+   +----------+   +--------------+        |
|  | COLLECT  |-->|  SEAL  |-->|  MATCH   |-->|  FINALIZE    |--+     |
|  |          |   |        |   |          |   |              |  |     |
|  | Accept   |   | Hash & |   | Pure     |   | Settle &     |  |     |
|  | orders   |   | freeze |   | compute  |   | consume SRs  |  |     |
|  | + escrow |   | batch  |   | trades   |   | + receipts   |  |     |
|  +----------+   +--------+   +----------+   +--------------+  |     |
|       ^                                                       |     |
|       +-------------------------------------------------------+     |
+---------------------------------------------------------------------+

+-----------------+   +-----------------+   +---------------------+
|  SECURITY       |   |  MATCHCORE      |   |  FINALITY           |
|  ENVELOPE       |   |  (Pure Compute) |   |  PLANE              |
|                 |   |                 |   |                     |
|  BalanceManager |   |  OrderBook      |   |  IdempotencyGuard   |
|  EscrowManager  |-->|  ClearingPrice  |-->|  Tier1Settler       |
|  RiskKernel     |   |  Matcher        |   |  SupplyConservation |
|  PendingBuffer  |   |  Determinism    |   |  WithdrawLock       |
|  BatchSealer    |   |                 |   |  Receipts           |
|                 |   |  ZERO SIDE      |   |                     |
|  Mints SRs      |   |  EFFECTS        |   |  Consumes SRs       |
+-----------------+   +-----------------+   +---------------------+
  openmatch-ingress    openmatch-matchcore    openmatch-settlement
```

### The Three Planes

**Security Envelope** (`openmatch-ingress`) -- The gatekeeper. Every order must
pass through balance verification, risk validation, and SpendRight minting
before it touches the matching engine. The escrow-first model ensures funds
are frozen *before* orders enter the book.

**MatchCore** (`openmatch-matchcore`) -- Pure deterministic computation. Takes a
`SealedBatch` (immutable, hash-committed set of orders) and produces a
`TradeBundle` (deterministic trades + Merkle trade root). No database writes.
No balance checks. No risk logic. No plugins. Same input = same output on
every node in the universe.

```rust
fn match_sealed_batch(batch: &SealedBatch) -> TradeBundle
```

**Finality Plane** (`openmatch-settlement`) -- Executes trades, consumes
SpendRights (ACTIVE -> SPENT), transfers balances, generates cryptographic
receipts, and verifies supply conservation. Settlement is idempotent -- the
same trade can never be settled twice.

### SpendRight: The Cryptographic Primitive

The **SpendRight** (SR) is OpenMatch's novel contribution to exchange design.
It is a cryptographic pre-commitment token that replaces traditional balance
freezing:

```
   +--------+  settlement   +-------+
   | ACTIVE |-------------->| SPENT |    (irreversible -- prevents double-spend)
   +---+----+               +-------+
       | cancel / expire
       v
   +----------+
   | RELEASED |                          (funds returned to user)
   +----------+
```

- **Atomic minting** -- SR created only when balance freeze succeeds
- **Single-use** -- ACTIVE -> SPENT is monotonic and irreversible
- **Nonce-bound** -- each SR has a unique nonce, preventing replay attacks
- **Signature-bound** -- signed by issuing node's ed25519 key
- **Time-bound** -- expires after epoch window

---

## Security Model

OpenMatch follows **Kerckhoffs's Principle**: the system is secure even with
full source code access. Security comes from cryptographic properties, not
obscurity.

| Layer | Guard | What It Prevents |
|-------|-------|-----------------|
| Ingress | **RiskKernel** | Oversized orders, price manipulation, order flooding |
| Ingress | **EscrowManager** | Unfunded orders (escrow-first model) |
| Ingress | **BatchSealer** | Input tampering (SHA-256 batch commitment) |
| MatchCore | **Self-Trade Prevention** | Wash trading (same user buy+sell) |
| MatchCore | **Determinism Verification** | Cross-node divergence (Merkle trade root) |
| Settlement | **IdempotencyGuard** | Double-settlement (LRU-bounded TradeId cache) |
| Settlement | **WithdrawLock** | Balance manipulation during Match/Finalize phases |
| Settlement | **SupplyConservation** | Phantom balances (sum check after every settlement) |
| SpendRight | **Monotonic State Machine** | Double-spend (ACTIVE->SPENT is irreversible) |

**Plugin Security**: Enterprise risk plugins can only **tighten** rules (throttle,
force Tier 3, cooldown, freeze) -- they can never weaken core safety invariants.

---

## Crate Map

```
openmatch/
|-- Cargo.toml                     # Workspace root (v0.2.0, edition 2024)
|-- crates/
|   |-- openmatch-types/           # Shared types, IDs, errors       (51 tests)
|   |   +-- src/
|   |       |-- ids.rs             # OrderId, UserId, SpendRightId, EpochId, TradeId
|   |       |-- order.rs           # Order, OrderSide, OrderType, OrderStatus
|   |       |-- trade.rs           # Trade (deterministic fill record)
|   |       |-- spend_right.rs     # SpendRight + state machine
|   |       |-- epoch.rs           # EpochPhase, SealedBatch, TradeBundle, BatchDigest
|   |       |-- balance.rs         # BalanceEntry (available/frozen)
|   |       |-- receipt.rs         # Receipt, ReceiptType (audit trail)
|   |       |-- error.rs           # OpenmatchError (OM_ERR_xxx codes)
|   |       |-- risk.rs            # RiskLimits, RiskDecision, AgentBinding
|   |       |-- config.rs          # NodeConfig, MarketConfig
|   |       +-- constants.rs       # System-wide limits and defaults
|   |
|   |-- openmatch-matchcore/       # Pure deterministic matcher      (40 tests)
|   |   +-- src/
|   |       |-- matcher.rs         # match_sealed_batch() -- THE core function
|   |       |-- orderbook.rs       # BTreeMap-based order book (bid/ask)
|   |       |-- price_level.rs     # FIFO price level with VecDeque
|   |       |-- clearing.rs        # Uniform clearing price computation
|   |       +-- determinism.rs     # Trade root hashing & verification
|   |
|   |-- openmatch-ingress/         # Security Envelope               (37 tests)
|   |   +-- src/
|   |       |-- balance_manager.rs # Available/frozen balance accounting
|   |       |-- escrow.rs          # SpendRight minting & release
|   |       |-- risk_kernel.rs     # Hard risk gate (fail-closed)
|   |       |-- pending_buffer.rs  # Order collection during COLLECT
|   |       +-- batch_sealer.rs    # Seal buffer -> SealedBatch + BatchDigest
|   |
|   +-- openmatch-settlement/      # Finality Plane                  (23+14 tests)
|       |-- src/
|       |   |-- tier1.rs           # Local atomic settlement
|       |   |-- idempotency.rs     # LRU-bounded double-settle prevention
|       |   |-- supply_conservation.rs  # Mathematical invariant checker
|       |   +-- withdraw_lock.rs   # Phase-aware withdrawal blocking
|       +-- tests/
|           +-- end_to_end.rs      # 14 cross-plane integration tests
|
|-- docs/                          # Architecture & design documents
|   |-- 00-ARCHITECTURE-DESIGN.md
|   |-- OpenMatch-HLD.md
|   |-- OpenMatch-LLD.md
|   +-- getting-started.md
|
+-- .prompt/                       # 18 system prompt files for AI-assisted dev
    |-- G0-workspace.md
    |-- C1-types.md ... C9-persistence.md
    |-- S1-contracts.md, S2-sdk.md
    |-- N1-market.md, N2-agent.md
    |-- T1-testing.md
    +-- SECURITY-fraud-protection.md
```

**Total: ~4,200 lines of Rust | 165 tests | 4 crates | 0 unsafe**

---

## Quick Start

### Prerequisites

- **Rust 1.85+** (edition 2024)
- **Cargo** (included with Rust)

### Build & Test

```bash
# Clone the repository
git clone https://github.com/openibank/OpenMatch.git
cd OpenMatch

# Build all crates
cargo build --workspace

# Run the full test suite (165 tests)
cargo test --workspace

# Run with verbose output
cargo test --workspace -- --nocapture

# Run clippy (pedantic linting enabled)
cargo clippy --workspace
```

### Use as a Library

Add to your `Cargo.toml`:

```toml
[dependencies]
openmatch-types      = { git = "https://github.com/openibank/OpenMatch" }
openmatch-matchcore  = { git = "https://github.com/openibank/OpenMatch" }
openmatch-ingress    = { git = "https://github.com/openibank/OpenMatch" }
openmatch-settlement = { git = "https://github.com/openibank/OpenMatch" }
```

For a complete usage guide, see [docs/getting-started.md](docs/getting-started.md).

---

## Design Principles

1. **Escrow-first** -- No order enters the book without frozen funds. Ever.
2. **Deterministic matching** -- Same `SealedBatch` -> same `TradeBundle` on every node.
3. **Monotonic state transitions** -- SpendRight states never go backwards.
4. **Fail-closed** -- If any security check errors, the action is rejected, not allowed.
5. **Supply conservation** -- Mathematical invariant checked after every settlement.
6. **Zero unsafe** -- The entire codebase forbids `unsafe` code at the workspace level.
7. **Plugin-safe** -- Plugins can only tighten rules, never weaken them.
8. **Audit trail** -- Every action produces a signed, hashchained `Receipt`.

---

## Error Codes

All errors use the `OM_ERR_` prefix for machine-parseable log analysis:

| Range | Subsystem | Example |
|-------|-----------|---------|
| 1xx | Orders | `OM_ERR_100` Order not found |
| 2xx | Balances | `OM_ERR_200` Insufficient balance |
| 3xx | SpendRight | `OM_ERR_300` Invalid SpendRight |
| 4xx | Epoch | `OM_ERR_400` Wrong epoch phase |
| 5xx | Matching | `OM_ERR_502` Self-trade blocked |
| 6xx | Settlement | `OM_ERR_602` Trade already settled |
| 7xx | Network | `OM_ERR_700` Node not found |
| 8xx | Security | `OM_ERR_801` Supply invariant violation |
| 9xx | Internal | `OM_ERR_900` Internal error |

---

## Roadmap

- [x] Core types and identifiers (UUIDv7, SpendRightId, EpochId)
- [x] SpendRight state machine with monotonic transitions
- [x] Pure deterministic batch matcher (zero side effects)
- [x] Self-trade prevention (wash trading blocked)
- [x] Uniform clearing price computation
- [x] Merkle trade root for cross-node verification
- [x] Security Envelope (ingress pipeline)
- [x] Tier 1 local atomic settlement
- [x] Settlement idempotency guard
- [x] Supply conservation invariant
- [x] Phase-aware withdraw lock
- [x] Risk kernel with per-epoch rate limiting
- [x] Batch sealing with SHA-256 commitment
- [x] 165 tests across 4 crates
- [ ] P2P gossip network (`openmatch-gossip`)
- [ ] REST/WebSocket API (`openmatch-api`)
- [ ] WAL + snapshot persistence (`openmatch-persistence`)
- [ ] Tier 2 cross-node settlement
- [ ] Tier 3 on-chain settlement
- [ ] Ed25519 signature verification (currently placeholder)
- [ ] Agent SDK for algorithmic trading
- [ ] CLI tooling

---

## Contributing

We welcome contributions from the community. Please read the architecture
docs in `docs/` before submitting PRs to understand the three-plane
separation and security model.

```bash
# Development workflow
cargo fmt --all                     # Format code
cargo clippy --workspace            # Lint (pedantic enabled)
cargo test --workspace              # Run all 165 tests
```

**Golden Rule**: MatchCore must remain pure. If your change adds a side effect
to the matching engine (DB write, network call, balance check), it belongs
in Ingress or Settlement, not MatchCore.

---

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option.

---

<p align="center">
  <strong>Built with conviction by the <a href="https://github.com/openibank">OpeniBank Research Team</a></strong>
  <br/>
  <em>"Fair markets through deterministic matching."</em>
</p>
