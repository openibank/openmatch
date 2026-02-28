//! Error types for the OpenMatch matching engine.
//!
//! All errors use the `OM_ERR_` prefix convention for easy grepping in logs.
//! Error codes are grouped by subsystem:
//! - 1xx: Order errors
//! - 2xx: Balance errors
//! - 3xx: SpendRight / escrow errors
//! - 4xx: Epoch errors
//! - 5xx: Matching errors
//! - 6xx: Settlement errors
//! - 7xx: Network errors
//! - 8xx: Security errors
//! - 9xx: General / internal errors

use rust_decimal::Decimal;
use thiserror::Error;

use crate::{EpochPhase, NodeId, OrderId};

/// Central error enum for all OpenMatch operations.
#[derive(Debug, Error)]
pub enum OpenmatchError {
    // =================================================================
    // Order Errors (1xx)
    // =================================================================
    /// The requested order was not found in the book or buffer.
    #[error("OM_ERR_100: Order not found: {0}")]
    OrderNotFound(OrderId),

    /// The order failed validation (missing fields, bad values, etc.).
    #[error("OM_ERR_101: Invalid order: {reason}")]
    InvalidOrder { reason: String },

    /// An order with this ID already exists.
    #[error("OM_ERR_102: Order already exists: {0}")]
    DuplicateOrder(OrderId),

    /// The order cannot be cancelled in its current state.
    #[error("OM_ERR_103: Order cannot be cancelled in current state")]
    OrderNotCancellable,

    /// Too many open orders for this user in this market.
    #[error("OM_ERR_104: Order limit exceeded for user")]
    OrderLimitExceeded,

    // =================================================================
    // Balance Errors (2xx)
    // =================================================================
    /// Not enough available balance to perform the operation.
    #[error("OM_ERR_200: Insufficient available balance: need {needed}, have {available}")]
    InsufficientBalance { needed: Decimal, available: Decimal },

    /// Not enough frozen balance to unfreeze or settle.
    #[error("OM_ERR_201: Insufficient frozen balance")]
    InsufficientFrozen,

    /// A balance operation would produce a negative value.
    #[error("OM_ERR_202: Balance underflow")]
    BalanceUnderflow,

    // =================================================================
    // SpendRight / Escrow Errors (3xx)
    // =================================================================
    /// The SpendRight is structurally invalid.
    #[error("OM_ERR_300: Invalid SpendRight: {reason}")]
    InvalidSpendRight { reason: String },

    /// The SpendRight has expired.
    #[error("OM_ERR_301: SpendRight expired")]
    SpendRightExpired,

    /// The ed25519 signature on the SpendRight didn't verify.
    #[error("OM_ERR_302: SpendRight signature verification failed")]
    SpendRightSignatureInvalid,

    /// Nonce was already used (replay attack prevention).
    #[error("OM_ERR_303: SpendRight nonce already used")]
    SpendRightNonceReused,

    // =================================================================
    // Epoch Errors (4xx)
    // =================================================================
    /// An operation was attempted in the wrong epoch phase.
    #[error("OM_ERR_400: Wrong epoch phase: expected {expected}, got {actual}")]
    WrongEpochPhase {
        expected: EpochPhase,
        actual: EpochPhase,
    },

    /// An epoch phase timed out.
    #[error("OM_ERR_401: Epoch timeout during {phase}")]
    EpochTimeout { phase: EpochPhase },

    /// The pending buffer has already been sealed for this epoch.
    #[error("OM_ERR_402: Pending buffer already sealed")]
    BufferAlreadySealed,

    /// The pending buffer is full (MAX_ORDERS_PER_BATCH reached).
    #[error("OM_ERR_403: Pending buffer full")]
    BufferFull,

    // =================================================================
    // Matching Errors (5xx)
    // =================================================================
    /// The matching algorithm encountered an error.
    #[error("OM_ERR_500: Matching failed: {reason}")]
    MatchingFailed { reason: String },

    /// Cross-node determinism check failed.
    #[error("OM_ERR_501: Determinism violation: expected {expected}, got {actual}")]
    DeterminismViolation { expected: String, actual: String },

    /// Self-trade detected and prevented (wash trading).
    #[error("OM_ERR_502: Self-trade prevented: buyer and seller are the same user")]
    SelfTradeBlocked,

    // =================================================================
    // Settlement Errors (6xx)
    // =================================================================
    /// Settlement of a trade failed.
    #[error("OM_ERR_600: Settlement failed: {reason}")]
    SettlementFailed { reason: String },

    /// The on-chain settlement transaction was rejected.
    #[error("OM_ERR_601: On-chain settlement rejected: {reason}")]
    OnChainRejected { reason: String },

    /// A trade has already been settled (idempotency guard).
    #[error("OM_ERR_602: Trade already settled: {0}")]
    TradeAlreadySettled(crate::TradeId),

    /// Withdrawals are locked during MATCH/FINALIZE phases.
    #[error("OM_ERR_603: Withdrawals locked during settlement")]
    WithdrawLockedDuringSettle,

    // =================================================================
    // Security Errors (8xx)
    // =================================================================
    /// Rate limit exceeded for this user.
    #[error("OM_ERR_800: Rate limit exceeded: {reason}")]
    RateLimitExceeded { reason: String },

    /// Supply conservation invariant violated — critical safety alert.
    #[error("OM_ERR_801: Supply invariant violation: {reason}")]
    SupplyInvariantViolation { reason: String },

    /// SpendRight nonce was already used (replay attack).
    #[error("OM_ERR_802: Nonce replay detected for node {node_hex} nonce {nonce}")]
    NonceReplay { node_hex: String, nonce: u64 },

    /// Order flood detected from a single user.
    #[error("OM_ERR_803: Order flood detected: {count} orders in {window_ms}ms from user")]
    OrderFloodDetected { count: usize, window_ms: u64 },

    /// Suspicious price deviation — potential market manipulation.
    #[error("OM_ERR_804: Suspicious price: {reason}")]
    SuspiciousPrice { reason: String },

    // =================================================================
    // Network Errors (7xx)
    // =================================================================
    /// A node was not found in the peer table.
    #[error("OM_ERR_700: Node not found: {0}")]
    NodeNotFound(NodeId),

    /// Gossip protocol error.
    #[error("OM_ERR_701: Gossip error: {reason}")]
    GossipError { reason: String },

    /// Peer connection failed.
    #[error("OM_ERR_702: Peer connection failed: {reason}")]
    PeerConnectionFailed { reason: String },

    // =================================================================
    // General / Internal (9xx)
    // =================================================================
    /// Unrecoverable internal error.
    #[error("OM_ERR_900: Internal error: {0}")]
    Internal(String),

    /// Serialization / deserialization error.
    #[error("OM_ERR_901: Serialization error: {0}")]
    Serialization(String),

    /// Configuration error (invalid config file, missing fields, etc.).
    #[error("OM_ERR_902: Configuration error: {0}")]
    Configuration(String),

    /// I/O error (disk, network).
    #[error("OM_ERR_903: I/O error: {0}")]
    Io(String),
}

/// Crate-wide `Result` alias.
pub type Result<T> = std::result::Result<T, OpenmatchError>;

// Conversion from std::io::Error
impl From<std::io::Error> for OpenmatchError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_contains_prefix() {
        let err = OpenmatchError::OrderNotFound(OrderId::new());
        let msg = format!("{err}");
        assert!(msg.starts_with("OM_ERR_100"), "Got: {msg}");
    }

    #[test]
    fn insufficient_balance_display() {
        let err = OpenmatchError::InsufficientBalance {
            needed: Decimal::new(100, 0),
            available: Decimal::new(50, 0),
        };
        let msg = format!("{err}");
        assert!(msg.contains("OM_ERR_200"));
        assert!(msg.contains("100"));
        assert!(msg.contains("50"));
    }

    #[test]
    fn wrong_epoch_phase_display() {
        let err = OpenmatchError::WrongEpochPhase {
            expected: EpochPhase::Collect,
            actual: EpochPhase::Match,
        };
        let msg = format!("{err}");
        assert!(msg.contains("OM_ERR_400"));
        assert!(msg.contains("COLLECT"));
        assert!(msg.contains("MATCH"));
    }

    #[test]
    fn all_errors_have_om_err_prefix() {
        let errors: Vec<Box<dyn std::error::Error>> = vec![
            Box::new(OpenmatchError::InsufficientFrozen),
            Box::new(OpenmatchError::BufferAlreadySealed),
            Box::new(OpenmatchError::SpendRightExpired),
            Box::new(OpenmatchError::Internal("test".into())),
            Box::new(OpenmatchError::DeterminismViolation {
                expected: "a".into(),
                actual: "b".into(),
            }),
        ];
        for err in errors {
            let msg = format!("{err}");
            assert!(
                msg.starts_with("OM_ERR_"),
                "Error missing OM_ERR_ prefix: {msg}"
            );
        }
    }
}
