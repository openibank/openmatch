//! # openmatch-ingress
//!
//! **Security Envelope Plane**: order ingress, SpendRight minting,
//! risk validation, pending buffer management, and batch sealing.
//!
//! ## Architecture
//!
//! The Security Envelope sits between the API layer and MatchCore:
//! 1. **BalanceManager**: tracks available/frozen balances per (user, asset)
//! 2. **EscrowManager**: freezes funds and mints SpendRights
//! 3. **RiskKernel**: hard gate — validates order against risk limits
//! 4. **PendingBuffer**: collects validated orders during COLLECT phase
//! 5. **BatchSealer**: seals the buffer into a `SealedBatch` + `BatchDigest`
//!
//! ## Order Flow
//!
//! ```text
//! API → BalanceManager.freeze() → RiskKernel.validate() → PendingBuffer.push()
//!     → BatchSealer.seal() → SealedBatch → MatchCore
//! ```
//!
//! Every order entering MatchCore **must** have a valid SpendRight.

pub mod balance_manager;
pub mod batch_sealer;
pub mod escrow;
pub mod pending_buffer;
pub mod risk_kernel;

pub use balance_manager::BalanceManager;
pub use batch_sealer::BatchSealer;
pub use escrow::EscrowManager;
pub use pending_buffer::PendingBuffer;
pub use risk_kernel::RiskKernel;
