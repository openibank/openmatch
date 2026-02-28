//! # openmatch-matchcore
//!
//! **Pure deterministic matching engine for OpenMatch.**
//!
//! MatchCore is the compute plane -- it takes a sealed batch of pre-funded
//! orders and produces a deterministic set of trades. It has:
//!
//! - **Zero side effects**: no DB writes, no balance checks, no risk logic
//! - **Deterministic output**: same input -> same output on every node
//! - **Self-trade prevention**: wash trading blocked at the match level
//! - **Market sharding**: each market has its own independent book

pub mod clearing;
pub mod determinism;
pub mod matcher;
pub mod orderbook;
pub mod price_level;

pub use clearing::{ClearingResult, compute_clearing_price};
pub use determinism::{compute_trade_root, verify_trade_root};
pub use matcher::match_sealed_batch;
pub use orderbook::OrderBook;
pub use price_level::PriceLevel;
