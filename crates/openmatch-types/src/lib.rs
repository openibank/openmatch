//! # openmatch-types
//!
//! Shared types, errors, and configuration for the **OpenMatch** matching engine.
//!
//! This crate is the leaf dependency of the workspace — every other crate
//! depends on it. It defines:
//!
//! - **Identifiers**: [`OrderId`], [`UserId`], [`NodeId`], [`TradeId`], [`EpochId`], [`SpendRightId`], [`MarketPair`]
//! - **Order model**: [`Order`], [`OrderSide`], [`OrderType`], [`OrderStatus`]
//! - **Trade model**: [`Trade`]
//! - **SpendRight model**: [`SpendRight`], [`SpendRightState`]
//! - **Receipt model**: [`Receipt`], [`ReceiptType`]
//! - **Epoch model**: [`EpochPhase`], [`EpochConfig`], [`SealedBatch`], [`TradeBundle`], [`BatchDigest`]
//! - **Balance model**: [`BalanceEntry`], [`Asset`]
//! - **Configuration**: [`NodeConfig`], [`NetworkConfig`], [`MarketConfig`]
//! - **Errors**: [`OpenmatchError`] with `OM_ERR_` prefix codes
//! - **Risk management**: [`RiskLimits`], [`RiskDecision`], [`AgentId`]
//! - **Constants**: system-wide limits and defaults

pub mod balance;
pub mod config;
pub mod constants;
pub mod epoch;
pub mod error;
pub mod ids;
pub mod order;
pub mod receipt;
pub mod risk;
pub mod spend_right;
pub mod trade;

// Re-export all primary types at crate root for ergonomic imports:
//   use openmatch_types::{Order, OrderSide, Trade, SpendRight, ...};

pub use balance::*;
pub use config::*;
pub use epoch::*;
pub use error::*;
pub use ids::*;
pub use order::*;
pub use receipt::*;
pub use risk::*;
pub use spend_right::*;
pub use trade::*;

// Constants are accessed via `openmatch_types::constants::FOO`
// (not re-exported to avoid name collisions).
