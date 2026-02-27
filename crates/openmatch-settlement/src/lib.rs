//! # openmatch-settlement
//!
//! **Finality Plane**: settlement execution, SpendRight consumption,
//! cryptographic receipts, and evidence generation.
//!
//! ## Architecture
//!
//! The Finality Plane receives a [`TradeBundle`] from MatchCore and:
//! 1. Validates idempotency (no double-settlement)
//! 2. Consumes SpendRights (ACTIVE → SPENT)
//! 3. Executes balance transfers (frozen → counterparty available)
//! 4. Generates cryptographic receipts for audit trail
//! 5. Checks supply conservation invariant
//!
//! ## 3-Tier Settlement
//!
//! - **Tier 1**: Local atomic (within same node) — instant
//! - **Tier 2**: Cross-node gossip settlement — sub-second
//! - **Tier 3**: On-chain finality — minutes/blocks

pub mod idempotency;
pub mod supply_conservation;
pub mod tier1;
pub mod withdraw_lock;

pub use idempotency::IdempotencyGuard;
pub use supply_conservation::SupplyConservation;
pub use tier1::Tier1Settler;
pub use withdraw_lock::WithdrawLock;
