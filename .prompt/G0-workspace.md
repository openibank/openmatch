# G0 - OpeniMatch Workspace Context

> **Match fair. Settle safe. Trust math.**

Include this file with every crate-level prompt. It provides the global context
that every implementor needs.

---

## 1. Project Mission

OpeniMatch is an **epoch-based decentralized matching engine** for digital asset
exchange. It replaces continuous matching with batch auctions to eliminate
front-running and ensure fairness. Every order must be escrowed before entering
the book, and trades settle through a 3-tier model (intra-node, cross-node
escrow-verified, on-chain).

Repository: `https://github.com/openibank/OpeniMatch`
License: MIT OR Apache-2.0

---

## 2. Workspace Layout

```
OpeniMatch/
├── Cargo.toml                    # Workspace root, resolver = "3"
├── rustfmt.toml                  # edition = "2024", max_width = 100
├── clippy.toml                   # msrv = "1.85"
├── .prompt/                      # AI agent prompts (this directory)
│   ├── G0-workspace.md           # This file (global context)
│   ├── C1-types.md               # openmatch-types crate prompt
│   ├── C2-core.md                # openmatch-core crate prompt
│   └── C3-epoch.md               # openmatch-epoch crate prompt
└── crates/
    ├── openmatch-types/          # DONE - Shared types, errors, configs
    ├── openmatch-core/           # DONE - Orderbook, batch matcher, balance mgr
    ├── openmatch-epoch/          # Epoch controller (async state machine)
    ├── openmatch-escrow/         # Escrow manager, freeze proofs
    ├── openmatch-settlement/     # 3-tier settlement engine
    ├── openmatch-receipt/        # Cryptographic receipts
    ├── openmatch-gossip/         # P2P gossip (libp2p)
    ├── openmatch-api/            # REST + WebSocket API
    ├── openmatch-agent/          # AI agent runtime
    ├── openmatch-persistence/    # PostgreSQL + AOF
    ├── openmatch-market/         # OHLCV, klines, ticker
    └── openmatch-cli/            # Node management CLI
```

Dependency flow (left depends on right):

```
cli -> api -> epoch -> core -> types
                    -> escrow -> types
                    -> settlement -> types
                    -> receipt -> types
gossip -> types
persistence -> types
market -> core -> types
agent -> api, core, types
```

---

## 3. The Epoch Mechanism

Every matching cycle is an **epoch**. Each epoch passes through three
non-overlapping phases:

```
┌──────────┐    ┌──────────┐    ┌──────────┐
│ COLLECT  │───>│  MATCH   │───>│  SETTLE  │──┐
│          │    │          │    │          │  │
│ Orders   │    │ Buffer   │    │ Trades   │  │
│ enter    │    │ sealed,  │    │ settle,  │  │
│ buffer   │    │ matched  │    │ balances │  │
│          │    │          │    │ transfer │  │
└──────────┘    └──────────┘    └──────────┘  │
      ^                                       │
      └───────────────────────────────────────┘
```

### Phase Details

| Phase   | Default Duration | What Happens |
|---------|-----------------|--------------|
| COLLECT | 1000 ms         | Orders flow into `PendingBuffer`. Each gets a monotonic sequence number. |
| MATCH   | 500 ms (timeout)| Buffer is sealed (sorted + SHA-256 hashed). `BatchMatcher` computes uniform clearing price and emits `Trade`s. |
| SETTLE  | 2000 ms (timeout)| Trades are settled via the 3-tier settlement model. Frozen balances transfer. |

Additional timing: `seal_grace` = 50 ms (late-arriving orders accepted within
this window after COLLECT ends).

**Critical invariant**: Phases NEVER overlap. COLLECT must finish before MATCH
begins; MATCH must finish before SETTLE begins.

---

## 4. Escrow-First Order Model

Every order **must** have a valid `FreezeProof` before entering the order book:

1. User requests order placement
2. Node freezes the required balance (available -> frozen)
3. Node issues a `FreezeProof` (ed25519-signed attestation)
4. Order + FreezeProof enter the `PendingBuffer`
5. If order is cancelled or expires, frozen balance returns to available
6. If order matches, frozen balance transfers to counterparty during settlement

### FreezeProof Structure

```
FreezeProof {
    order_id:     OrderId,
    user_id:      UserId,
    asset:        String,        // "USDT" for buys, "BTC" for sells
    amount:       Decimal,
    issuer_node:  NodeId,        // ed25519 public key
    signature:    Vec<u8>,       // ed25519 signature
    nonce:        u64,           // replay prevention
    created_at:   DateTime<Utc>,
    expires_at:   DateTime<Utc>,
}
```

Signing payload: `order_id(16) || user_id(16) || asset(utf8) || amount(str) || nonce(8)`

---

## 5. 3-Tier Settlement Model

| Tier | Name | When | Mechanism |
|------|------|------|-----------|
| 1 | Intra-node | Both users on same node | Direct in-memory balance transfer. Fastest path. |
| 2 | Escrow-verified | Users on different nodes | Cross-node verification: both nodes verify freeze proofs, then release funds via signed attestations. |
| 3 | On-chain | Dispute or periodic reconciliation | Smart contract arbitration. Freeze proofs are submitted on-chain as evidence. |

Settlement always attempts Tier 1 first, falling back to Tier 2 then Tier 3.

---

## 6. Deterministic Matching

The core invariant: **same sealed buffer produces the same trades on every node**.

This requires:
- Deterministic sort order in `PendingBuffer::seal()`: by (side, price_priority, sequence)
- Deterministic clearing price computation (maximize volume, tie-break by imbalance then higher price)
- Deterministic trade ID generation: `TradeId::deterministic(batch_id, fill_sequence)`
- SHA-256 domain-separated hashing for both input (`batch_hash`) and output (`result_hash`)
- Domain separator prefixes: `"openmatch:batch:v1:"`, `"openmatch:result:v1:"`, `"openmatch:trade_id:v1:"`

Cross-node verification: after matching, nodes gossip their `result_hash`. If hashes differ, a determinism violation error (`OM_ERR_501`) is raised.

---

## 7. Eleven Architectural Invariants

These are non-negotiable rules. Every crate must respect all of them.

1. **Epoch phases never overlap.** COLLECT must complete before MATCH starts; MATCH before SETTLE.
2. **No order without a FreezeProof.** Every `Order` struct has a `freeze_proof: FreezeProof` field. No exceptions.
3. **Deterministic matching.** Same sealed buffer -> same `result_hash` on every node.
4. **Escrow-first.** Balance is frozen (available -> frozen) before the order enters the book.
5. **Uniform clearing price.** All trades in a batch execute at the same price.
6. **Price-time priority.** Within the clearing price, orders fill in sequence order (earlier sequence fills first).
7. **No floating point for money.** All prices, quantities, and balances use `rust_decimal::Decimal`.
8. **All hashes are SHA-256 with domain separation.** Prefix with `"openmatch:{purpose}:v1:"`.
9. **All signatures are ed25519.** Via `ed25519-dalek` crate.
10. **No unsafe code.** `unsafe_code = "forbid"` in workspace lints.
11. **All IDs are UUIDv7** (except `NodeId` which is the ed25519 public key, and `BatchId`/`EpochId` which are monotonic u64).

---

## 8. Resonance Flow

The design philosophy follows a resonance pattern:

```
Presence -> Coupling -> Meaning -> Intent -> Commitment -> Consequence
```

| Stage | In OpeniMatch |
|-------|---------------|
| Presence | User connects to a node |
| Coupling | User deposits funds, establishing a balance relationship |
| Meaning | User observes orderbook state, market data |
| Intent | User submits an order (expresses desire to trade) |
| Commitment | FreezeProof issued, balance escrowed (skin in the game) |
| Consequence | Trade executes, settlement transfers funds (irreversible outcome) |

This flow ensures every participant has genuine commitment (escrowed funds)
before their intent can affect the market.

---

## 9. Tech Stack

| Component | Version / Crate | Purpose |
|-----------|----------------|---------|
| Language | Rust 2024 edition | `edition = "2024"` in Cargo.toml |
| MSRV | 1.85 | Minimum supported Rust version |
| Async runtime | `tokio 1.43` | `features = ["full"]` |
| Serialization | `serde 1.0` | `features = ["derive"]` |
| JSON | `serde_json 1.0` | API serialization |
| Decimal math | `rust_decimal 1.36` | `features = ["serde-with-str"]` |
| Cryptography | `ed25519-dalek 2.1` | `features = ["serde", "rand_core"]` |
| IDs | `uuid 1.11` | `features = ["v7", "serde"]` |
| Errors | `thiserror 2.0` | Derive `Error` trait |
| Time | `chrono 0.4` | `features = ["serde"]` |
| Logging | `tracing 0.1` / `tracing-subscriber 0.3` | Structured logging |
| Hashing | `sha2 0.10` | SHA-256 for batch/result hashes |
| Hex encoding | `hex 0.4` | Display of hashes and node IDs |
| RNG | `rand 0.8` | Test helpers, nonce generation |

### Future dependencies (not yet in workspace)

| Component | Planned Crate | Purpose |
|-----------|--------------|---------|
| P2P networking | `libp2p` | Gossip protocol |
| Protobuf | `prost` | Wire format for gossip messages |
| Database | `sqlx` + PostgreSQL | Persistence layer |
| HTTP | `axum` | REST API |
| WebSocket | `axum` + `tokio-tungstenite` | Real-time feeds |

---

## 10. Service Ports

| Port  | Service |
|-------|---------|
| 9000  | REST API (`/api/v1/`) |
| 9010  | WebSocket (`/ws/v1/`) |
| 9020  | gRPC (internal node-to-node) |
| 9030  | Metrics (Prometheus) |
| 9040  | Health check |
| 9050  | Admin API |
| 9060  | Debug / pprof |
| 9070  | Gossip protocol (libp2p) |
| 9080  | Reserved |
| 9944  | Default gossip port (constants) |
| 5432  | PostgreSQL |
| 6379  | Redis (caching, pub/sub) |

---

## 11. Naming Conventions

### Crate names
- Pattern: `openmatch-{component}` (kebab-case)
- Rust module: `openmatch_{component}` (snake_case)
- Examples: `openmatch-types`, `openmatch-core`, `openmatch-epoch`

### Error codes
- Pattern: `OM_ERR_{NNN}`
- Ranges:
  - 1xx: Order errors
  - 2xx: Balance errors
  - 3xx: Freeze / escrow errors
  - 4xx: Epoch errors
  - 5xx: Matching errors
  - 6xx: Settlement errors
  - 7xx: Network errors
  - 9xx: General / internal errors

### API paths
- REST: `/api/v1/{resource}`
- WebSocket: `/ws/v1/{channel}`

### Type naming
- IDs: `{Entity}Id` (e.g., `OrderId`, `UserId`, `TradeId`, `BatchId`, `EpochId`)
- Enums: `{Entity}{Property}` (e.g., `OrderSide`, `OrderType`, `OrderStatus`, `EpochPhase`)
- Configs: `{Scope}Config` (e.g., `EpochConfig`, `NodeConfig`, `NetworkConfig`, `MarketConfig`)

---

## 12. Coding Standards

### Mandatory rules

1. **No `unsafe` code.** The workspace sets `unsafe_code = "forbid"`.
2. **Clippy pedantic.** All crates inherit `clippy::pedantic = "warn"` from workspace lints.
   - Allowed exceptions: `module_name_repetitions`, `must_use_candidate`, `missing_errors_doc`, `missing_panics_doc`
3. **`rust_decimal::Decimal` for all money.** No `f32`, `f64`, or integer cents for prices, quantities, or balances.
4. **Serde derives on all public types.** `#[derive(Serialize, Deserialize)]` on every type that crosses a crate boundary.
5. **`#[must_use]` on query methods.** Any `fn` that returns a value without side effects.
6. **Domain-separated hashing.** Every SHA-256 hash starts with `"openmatch:{purpose}:v1:"`.
7. **UUIDv7 for entity IDs.** Time-ordered, lexicographically sortable.
8. **`Ord + PartialOrd`** on any type used as a `BTreeMap` key.

### Formatting (rustfmt.toml)

```toml
edition = "2024"
max_width = 100
use_field_init_shorthand = true
imports_granularity = "Crate"
group_imports = "StdExternalCrate"
```

### Cargo.toml patterns

- All crates use `version.workspace = true`, `edition.workspace = true`, etc.
- Dependencies reference `workspace = true` for shared versions.
- Each crate has `[lints] workspace = true`.
- Feature flags: `test-helpers` for test-only constructors (e.g., `FreezeProof::dummy`).

### Test conventions

- Unit tests: `#[cfg(test)] mod tests` inside each source file.
- Integration tests: `tests/` directory in each crate.
- Test helpers gated behind `#[cfg(any(test, feature = "test-helpers"))]`.
- Helper functions: `dec(n: i64) -> Decimal` for concise test values.
- Serde roundtrip tests for all serializable types.
- Every public error variant must have a display test confirming `OM_ERR_` prefix.

### Workspace build profiles

```toml
[profile.release]
lto = true
codegen-units = 1
strip = "symbols"

[profile.bench]
lto = true
codegen-units = 1
```

---

## 13. How to Add a New Crate

1. Create `crates/openmatch-{name}/Cargo.toml` inheriting workspace settings.
2. Add to `workspace.members` in root `Cargo.toml`.
3. Add to `workspace.dependencies` if other crates will depend on it.
4. Set `[lints] workspace = true`.
5. Add `test-helpers` feature if the crate has test-only constructors.
6. Ensure all public types derive `Serialize, Deserialize`.
7. Run `cargo clippy --workspace -- -D warnings` and `cargo test --workspace` before committing.

---

## 14. Current State

| Crate | Status | Tests |
|-------|--------|-------|
| openmatch-types | Done | 30+ unit tests |
| openmatch-core | Done | 70+ unit tests, 6 integration tests |
| openmatch-epoch | Not started | -- |
| openmatch-escrow | Not started | -- |
| openmatch-settlement | Not started | -- |
| openmatch-receipt | Not started | -- |
| All others | Not started | -- |

Combined test count: 107 passing tests across types and core.
