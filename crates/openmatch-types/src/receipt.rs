//! Cryptographic receipt types for the OpenMatch audit trail.
//!
//! Every significant action (order accepted, trade executed, settlement
//! completed) produces a signed [`Receipt`] that can be independently verified.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{EpochId, NodeId, TradeId};

/// The type of action this receipt proves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReceiptType {
    /// An order was accepted into the pending buffer.
    OrderAccepted,
    /// An order was rejected (invalid SpendRight, insufficient balance, etc.).
    OrderRejected,
    /// A trade was executed during batch matching.
    TradeExecuted,
    /// Settlement completed for a trade.
    SettlementCompleted,
    /// A SpendRight was minted (funds frozen for an order).
    SpendRightMinted,
    /// A SpendRight was released (order cancelled or SR expired).
    SpendRightReleased,
    /// A SpendRight was consumed (settlement consumed the SR).
    SpendRightSpent,
}

impl std::fmt::Display for ReceiptType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OrderAccepted => write!(f, "ORDER_ACCEPTED"),
            Self::OrderRejected => write!(f, "ORDER_REJECTED"),
            Self::TradeExecuted => write!(f, "TRADE_EXECUTED"),
            Self::SettlementCompleted => write!(f, "SETTLEMENT_COMPLETED"),
            Self::SpendRightMinted => write!(f, "SPEND_RIGHT_MINTED"),
            Self::SpendRightReleased => write!(f, "SPEND_RIGHT_RELEASED"),
            Self::SpendRightSpent => write!(f, "SPEND_RIGHT_SPENT"),
        }
    }
}

/// A cryptographically signed receipt proving that an action occurred.
///
/// Receipts form an append-only audit trail. Each receipt includes:
/// - A SHA-256 hash of the payload
/// - An ed25519 signature from the issuing node
/// - The epoch context in which the action occurred
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    /// What kind of action this receipt proves.
    pub receipt_type: ReceiptType,
    /// The epoch in which this action occurred.
    pub epoch_id: EpochId,
    /// The associated trade ID, if applicable.
    pub trade_id: Option<TradeId>,
    /// Opaque payload (serialized trade, order, settlement proof, etc.).
    pub payload: Vec<u8>,
    /// SHA-256 hash of the payload.
    pub payload_hash: [u8; 32],
    /// Ed25519 signature over `payload_hash` from the issuing node.
    pub signature: Vec<u8>,
    /// The node that issued this receipt.
    pub issuer_node: NodeId,
    /// When this receipt was issued.
    pub issued_at: DateTime<Utc>,
}

impl Receipt {
    /// Construct the bytes that should be signed: the payload hash.
    #[must_use]
    pub fn signing_bytes(&self) -> &[u8; 32] {
        &self.payload_hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_type_display() {
        assert_eq!(format!("{}", ReceiptType::TradeExecuted), "TRADE_EXECUTED");
        assert_eq!(
            format!("{}", ReceiptType::SettlementCompleted),
            "SETTLEMENT_COMPLETED"
        );
        assert_eq!(
            format!("{}", ReceiptType::SpendRightMinted),
            "SPEND_RIGHT_MINTED"
        );
    }

    #[test]
    fn receipt_type_serde_roundtrip() {
        let rt = ReceiptType::SpendRightMinted;
        let json = serde_json::to_string(&rt).unwrap();
        let back: ReceiptType = serde_json::from_str(&json).unwrap();
        assert_eq!(rt, back);
    }
}
