//! # openmatch-core
//!
//! Core matching engine for the **OpeniMatch** epoch-based batch auction system.
//!
//! This crate provides the core components plus security hardening:
//!
//! - [`OrderBook`]: BTreeMap-based order book with price-time priority
//! - [`PendingBuffer`]: Collects orders during COLLECT phase, seals for matching
//! - [`BatchMatcher`]: Deterministic batch matching with uniform clearing price
//! - [`BalanceManager`]: Per-user per-asset balance ledger with freeze/unfreeze
//! - [`security`]: Open-source-resistant security hardening module
//!
//! ## Security Philosophy (Kerckhoffs's Principle)
//!
//! All security relies on mathematical invariants and cryptographic proofs.
//! An attacker with full source code access **cannot** bypass these defenses
//! because they are based on:
//!
//! 1. **Double-spend prevention** — settlement idempotency (like blockchain UTXO)
//! 2. **Escrow-first model** — funds frozen before trading (like blockchain pre-commitment)
//! 3. **Deterministic execution** — every node produces identical results (like blockchain consensus)
//! 4. **Supply conservation** — mathematical proof that total supply is constant
//! 5. **Self-trade prevention** — wash trading blocked at the matching level
//!
//! ## Epoch Lifecycle
//!
//! ```text
//! ┌──────────┐    ┌──────────┐    ┌──────────┐
//! │ COLLECT  │───▶│  MATCH   │───▶│  SETTLE  │──┐
//! │          │    │          │    │          │  │
//! │ Orders → │    │ Buffer   │    │ Trades → │  │
//! │ Buffer   │    │ sealed,  │    │ Balance  │  │
//! │          │    │ matched  │    │ transfer │  │
//! └──────────┘    └──────────┘    └──────────┘  │
//!       ▲                                       │
//!       └───────────────────────────────────────┘
//! ```

pub mod balance_manager;
pub mod batch_matcher;
pub mod clearing;
pub mod orderbook;
pub mod pending_buffer;
pub mod price_level;
pub mod security;

pub use balance_manager::BalanceManager;
pub use batch_matcher::{BatchMatcher, BatchResult};
pub use clearing::{compute_clearing_price, ClearingResult};
pub use orderbook::OrderBook;
pub use pending_buffer::PendingBuffer;
pub use security::{
    NonceTracker, OrderRateLimiter, PriceSanityChecker, SecuredBalanceManager,
    SettlementIdempotencyGuard, SupplyConservation, WithdrawLock,
};
