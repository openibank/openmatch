# OpenMatch — Software High Level Design (HLD)

**Version:** v0.1  
**Date:** 2026-02-26  
**Project:** OpenMatch  
**Tagline:** Match fair. Settle safe. Trust math.

---

## 0) Two Architecture Graphs (TXT)

### Graph A — System Architecture (Nodes, Planes, and Tiers)

                    ┌──────────────────────────────────────────────┐
                    │                 Clients / Bots               │
                    │  Web UI / API Clients / Market Makers / AI   │
                    └───────────────────────┬──────────────────────┘
                                            │ REST/WS
                                 ┌──────────▼───────────┐
                                 │   openmatch-api/ws   │
                                 │  :9001 / :9002       │
                                 └──────────┬───────────┘
                                            │ internal RPC / calls
                                            │
┌───────────────────────────────────────────▼────────────────────────────────────────────────┐
│                                        OpenMatch Node                                      │
│                                                                                            │
│  ┌──────────────────────┐   ┌────────────────────────┐   ┌───────────────────────┐         │
│  │  Epoch Controller    │   │     Matching Core      │   │    Settlement Engine  │         │
│  │  (openmatch-epoch)   │   │   (openmatch-core)     │   │ (openmatch-settlement)│         │
│  │  COLLECT/MATCH/…     │   │   orderbook + match    │   │ Tier1/Tier2/Tier3     │         │
│  └──────────┬───────────┘   └───────────┬────────────┘   └───────────┬───────────┘         │
│             │                           │                            │                     │
│             │                           │                            │                     │
│  ┌──────────▼───────────┐   ┌───────────▼────────────┐   ┌───────────▼───────────┐         │
│  │ Pending Buffer       │   │ Escrow / Balance Freeze│   │ Receipts / Evidence   │         │
│  │ (epoch input batch)  │   │   (openmatch-escrow)   │   │ (openmatch-receipt)   │         │
│  │ orders+cancels+proof │   │ freeze_proof signing   │   │ signed receipt / WLL  │         │
│  └──────────┬───────────┘   └───────────┬────────────┘   └───────────┬───────────┘         │
│             │                           │                            │                     │
│             │                           │                            │                     │
│  ┌──────────▼──────────┐    ┌───────────▼───────────┐    ┌───────────▼───────────┐         │
│  │ Persistence         │    │ Market Data           │    │ Admin / Ops           │         │
│  │ (Postgres+AOF+snap) │    │ (OHLCV / ticker)      │    │ :9080                 │         │
│  └──────────┬──────────┘    └───────────────────────┘    └───────────────────────┘         │
│             │                                                                              │
└─────────────┼──────────────────────────────────────────────────────────────────────────────┘
              │
              │ libp2p gossip / mesh (:9000)
              │
┌─────────────▼──────────┐        ┌────────────────────────┐        ┌────────────────────────┐
│     Peer Node A        │<──────>│     Peer Node B        │<──────>│     Peer Node C        │
│ (same OpenMatch stack) │        │ (same OpenMatch stack) │        │ (same OpenMatch stack) │
└────────────────────────┘        └────────────────────────┘        └────────────────────────┘

Settlement tiers (per trade / per node-pair policy):
Tier 1: Intra-node atomic DB tx
Tier 2: Cross-node escrow-verified dual-sign receipts (challenge window)
Tier 3: On-chain escrow contract (fully trustless fallback)

---

### Graph B — Epoch Pipeline / Phase Isolation (No Overlap)

Time ───────────────────────────────────────────────────────────────────────────►

Epoch k (default 500ms):
┌──────────────────────────────┬───────────────────────┬───────────────────────┐
│          COLLECT             │        MATCH          │        SETTLE         │
│  (~300ms configurable)       │   (~100ms)            │   (~100ms)            │
└──────────────────────────────┴───────────────────────┴───────────────────────┘
- Accept orders/cancels            - Book LOCKED            - Execute settlement
- Verify signatures                - Seal batch             - Emit receipts/events
- Require freeze_proof             - Deterministic match    - Broadcast WS updates
- Gossip batch items               - Produce trades         - Clear pending buffer

Invariant: phases never overlap; no new orders mutate the book during MATCH.

---

## 1) Executive Summary

OpenMatch is a decentralized multi-node crypto matching and settlement system built to avoid:
- **Race conditions / double-fills** from continuous distributed matching
- **Settlement fraud / cheating** in cross-node fills
- **Mandatory on-chain trading** as the only trustless approach

OpenMatch achieves this through:
- **Epoch-based batch auction matching** (COLLECT → MATCH → SETTLE), with strict phase isolation
- **Escrow-first order acceptance**: every order includes a signed `freeze_proof`
- **Tiered settlement**: local fast path + cross-node escrow verify + on-chain escrow fallback

---

## 2) Goals, Non-Goals, and Hard Invariants

### 2.1 Goals
- Deterministic matching across nodes given the same epoch input batch.
- No orderbook mutation during MATCH (book lock).
- Settlement safety: match only what is escrowed (freeze_proof required).
- On-chain optionality: on-chain is **a settlement tier**, not a system requirement.

### 2.2 Non-Goals
- Full exchange UI/portal design (handled by OpeniBank web).
- Tokenomics / liquidity programs.
- “Global oracle truth” problem (market data is provided, but not a primary objective).

### 2.3 Hard Invariants
1. Phases never overlap (no pipelining of MATCH with COLLECT).
2. MATCH executes on a sealed input batch (batch hash / merkle root).
3. Orders without valid `freeze_proof` never enter the pending buffer.
4. Settlement is pluggable; matching output is settlement-agnostic.

---

## 3) System Context and Deployment Modes

### 3.1 Node and Service Layout (Crate-Level)
- `openmatch-core` — orderbook + deterministic batch matcher
- `openmatch-epoch` — epoch controller (phase scheduler)
- `openmatch-escrow` — balance freeze + freeze_proof
- `openmatch-settlement` — tiered settlement engine
- `openmatch-receipt` — signed receipts + evidence stream
- `openmatch-gossip` — libp2p mesh and message propagation
- `openmatch-api` — REST + WebSocket
- `openmatch-persistence` — PostgreSQL + AOF + snapshots
- `openmatch-market` — OHLCV/klines/ticker generation
- `openmatch-cli` — ops tooling

### 3.2 Ports (Reserved)
- `:9000` node engine + gossip
- `:9001` REST
- `:9002` WebSocket
- `:9080` admin
- `:5432` PostgreSQL
- `:6379` Redis

### 3.3 Deployment Modes
- **Standalone:** single node, epoch matching, Tier 1 only (dev/test)
- **Network:** multi-node mesh, epoch synced, Tier 1+2 (default)
- **Federated:** permissioned nodes, Tier 1+2 (regulated)
- **Trustless:** network + Tier 3 on-chain escrow (adversarial)

---

## 4) High-Level Architecture

### 4.1 Planes
- **Trading plane:** order intake → pending buffer → batch match → trade list
- **Settlement plane:** escrow verification → balance movements → finalization proofs
- **Evidence plane:** receipts + audit events (for dispute resolution and post-trade audit)

### 4.2 Matching Output Contract
Matching produces an **append-only list** of trades with deterministic IDs:
- `epoch_id`
- `batch_hash`
- `trade_id`
- `maker_order_id`
- `taker_order_id`
- `price`
- `quantity`
- `fees`
- `settlement_hint` (tier suggestion, not mandate)

---

## 5) Epoch-Based Batch Auction Matching

### 5.1 Phase Semantics
- **COLLECT**
  - accept new orders and cancels
  - validate request signatures
  - validate `freeze_proof` for any order that reserves funds
  - gossip accepted items to peers
  - *book is readable, but mutating inserts wait for MATCH*

- **MATCH**
  - lock book (no external mutation)
  - seal pending buffer → compute `batch_hash` (and optionally merkle root)
  - deterministic insertion + matching
  - produce trades + updated book state

- **SETTLE**
  - execute settlement for each trade (tier policy)
  - emit receipts and evidence
  - broadcast results (WS)
  - clear pending buffer; advance epoch

### 5.2 Determinism Strategy
Determinism relies on:
- identical sealed batch contents (orders/cancels/proofs)
- a canonical ordering rule (sort key) for batch processing
- identical market config (tick size, lot size, precision)
- deterministic trade ID derivation (hash of epoch_id + maker/taker + sequence)

---

## 6) Escrow-First Order Model (Anti-Fraud Core)

### 6.1 Freeze Proof Structure (Logical)

FreezeProof:
user_id
asset
amount
nonce
epoch_id (or valid_until_epoch)
node_id
node_signature (Ed25519)

### 6.2 Escrow-First Intake Flow
1) user signs order request (client signature)
2) node verifies signature and account status
3) node checks available balance
4) node **freezes** required funds (escrow bucket)
5) node generates and signs `freeze_proof`
6) order + `freeze_proof` enters pending buffer and is gossiped
7) peers verify node signature on `freeze_proof` before accepting into their pending view

### 6.3 Why This Prevents Cheating
- An order cannot be “sprayed” across nodes without reserved funds.
- Settlement feasibility becomes a precondition to matching.
- In cross-node trades, both legs are pre-reserved before any settlement execution begins.

---

## 7) Tiered Settlement Model

### 7.1 Tiers
- **Tier 1: Intra-node**
  - both counterparties on same node
  - single atomic DB transaction moves balances
  - node-signed receipt

- **Tier 2: Escrow-verified cross-node**
  - both legs have valid `freeze_proof`
  - each node releases its own frozen leg upon confirmation
  - dual-signed receipt (or multi-sig receipt set)
  - optional challenge window for disputes

- **Tier 3: On-chain escrow**
  - funds are held in a smart contract escrow
  - match proof submitted and verified on-chain
  - atomic release by contract; receipt is on-chain tx proof

### 7.2 Policy (When to Use Tier 3)
Tier 3 is recommended when:
- counterparties are unknown / high risk
- regulatory requirement mandates on-chain settlement evidence
- dispute arbitration requires trustless enforcement
- user explicitly opts into on-chain guarantees

---

## 8) Components and Responsibilities

### 8.1 `openmatch-epoch`
- epoch clock / scheduler
- strict phase boundaries
- batch sealing + batch hash
- timeouts and phase overruns monitoring

### 8.2 `openmatch-core`
- in-memory orderbook
- deterministic batch insert
- deterministic matching algorithm
- produces trades + updated book state

### 8.3 `openmatch-escrow`
- balance manager with freeze buckets
- generates/verifies freeze_proof signatures
- enforces “no valid proof, no order”

### 8.4 `openmatch-settlement`
- selects tier based on node-pair policy and trade attributes
- executes settlement (local/cross-node/on-chain adapter)
- produces settlement confirmations

### 8.5 `openmatch-receipt`
- receipt signing and verification
- dual-sign receipt support
- evidence stream emission (audit trail)

### 8.6 `openmatch-gossip`
- peer discovery and membership
- gossips epoch inputs during COLLECT
- syncs epoch parameters and peer liveness

### 8.7 `openmatch-api/ws`
- REST for trading actions and queries
- WS for market streams and receipts
- backpressure and authentication policies

### 8.8 `openmatch-persistence`
- PostgreSQL for durable ledger state
- AOF for event replay
- snapshots for fast recovery

---

## 9) Underlying Matching Logic Diagram (TXT)

### Diagram — Deterministic Batch Matching (Price-Time Priority)

Inputs:
	•	Book (state at end of prior epoch)
	•	PendingBuffer (sealed at MATCH start):
Orders: O1..On (buy/sell, price, qty, ts, user, freeze_proof, …)
Cancels: C1..Cm (order_id)
	•	MarketConfig (tick/lot/precision)

MATCH Phase (deterministic):
	1.	LOCK book
	2.	Seal pending buffer:
batch_hash = H(epoch_id || canonical_encode(PendingBuffer))
	3.	Apply cancels (canonical order):
for cancel in sort(Cancels by (order_id asc)):
remove order_id from book if present
	4.	Insert orders into book (canonical order):
sort orders by:
(side, price aggressiveness, ts, order_id)
NOTE: canonical tie-breakers must be total-order.
Insert into price levels (BTreeMap<price, FIFO queue>)
	5.	Run match loop:
while best_bid_price >= best_ask_price:
maker = top_of_best_price_level(older by FIFO)
taker = opposite side order crossing the spread
fill_qty = min(maker.remain, taker.remain)
trade_price = maker.price (or rule: maker price)
emit Trade(epoch_id, seq++, maker_id, taker_id, price, fill_qty)
decrement remains; remove if remain==0
	6.	Produce outputs:
	•	Trades[]
	•	New book state snapshot (or diff)
	•	batch_hash + trade_root (optional merkle of trades)
	7.	UNLOCK book → transition to SETTLE

**Important:** OpenMatch is epoch/batch oriented. You do not accept “continuous match” while matching; intake continues only during COLLECT and is buffered.

---

## 10) Key Flows

### 10.1 Intra-node (Tier 1)
- COLLECT: place orders → freeze → pending
- MATCH: sealed batch → deterministic trades
- SETTLE: atomic DB tx moves balances → receipt → WS broadcast

### 10.2 Cross-node (Tier 2)
- both nodes freeze locally and gossip orders+proofs
- deterministic trade computed from sealed batch
- SETTLE: each node releases escrow leg; exchange confirmations; dual-sign receipt; optional challenge window

### 10.3 On-chain (Tier 3)
- escrow on-chain
- submit match proof
- contract verifies signatures and releases atomically

---

## 11) Security Model (HLD)

### 11.1 Threats Addressed
- distributed race windows / double fills → epoch lock + sealed batch
- settlement cheating → escrow-first + proof verification + dual-sign receipts
- MEV / latency advantage → reduced via batch auctions and uniform crossing

### 11.2 Evidence & Accountability
- escrow freeze is the accountability boundary
- receipts are the verifiable artifact (node-signed / dual-signed / on-chain)

---

## 12) Observability and Ops

### 12.1 Metrics
- epoch duration and phase durations
- pending batch size and sealing hash agreement rate
- match latency and fills per epoch
- settlement success rate by tier
- dispute/challenge counts and resolution times

### 12.2 Logs / Correlation
- keys: `epoch_id`, `batch_hash`, `order_id`, `trade_id`, `receipt_id`

### 12.3 Admin Controls (Guarded)
- epoch configuration
- peer allowlist (federated)
- settlement policy overrides per node-pair / market

---

## 13) Public Interfaces (HLD-Level)

### 13.1 REST (proposed)
- `POST /api/v1/orders`
- `POST /api/v1/orders/cancel`
- `GET  /api/v1/orderbook/{market}`
- `GET  /api/v1/trades/{market}`
- `GET  /api/v1/receipts/{receipt_id}`
- `GET  /api/v1/epoch/status`

### 13.2 WebSocket (proposed)
- `trades:{market}`
- `orderbook:{market}`
- `receipts:{account|node}`
- `epoch:{global|node}`

---

## 14) Roadmap (Implementation Order)
1) types & schemas
2) core orderbook + matcher
3) epoch controller
4) escrow + freeze_proof
5) settlement tier framework
6) receipts + evidence stream
7) REST/WS APIs
8) persistence (AOF + snapshots)
9) gossip mesh + batch agreement
10) disputes / challenge window
11) on-chain escrow adapter + contracts
12) SDKs + benchmarks + compliance hardening

---

