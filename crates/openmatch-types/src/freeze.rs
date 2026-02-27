//! Freeze proof types for the OpeniMatch escrow-first model.
//!
//! Every order **must** have a valid `FreezeProof` before entering the book.
//! A freeze proof is a cryptographic attestation that the required balance
//! has been frozen (escrowed) on the issuing node.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{NodeId, OrderId, UserId};

/// Cryptographic proof that a user's balance has been frozen (escrowed)
/// for a specific order. Without a valid `FreezeProof`, an order is
/// rejected before it enters the order book.
///
/// # Security Properties
/// - Signed by the issuing node's ed25519 key
/// - Includes a nonce to prevent replay attacks
/// - Has an expiry time; stale proofs are rejected
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FreezeProof {
    /// The order this proof is for.
    pub order_id: OrderId,
    /// The user whose balance was frozen.
    pub user_id: UserId,
    /// The asset that was frozen (e.g., "USDT" for a buy, "BTC" for a sell).
    pub asset: String,
    /// Amount frozen.
    pub amount: Decimal,
    /// The node that issued this freeze.
    pub issuer_node: NodeId,
    /// Ed25519 signature over the canonical signing payload.
    pub signature: Vec<u8>,
    /// Nonce to prevent replay.
    pub nonce: u64,
    /// When the freeze was created.
    pub created_at: DateTime<Utc>,
    /// When the freeze expires (order must match before this).
    pub expires_at: DateTime<Utc>,
}

impl FreezeProof {
    /// Construct the canonical bytes that were signed.
    ///
    /// Format: `order_id(16) || user_id(16) || asset(utf8) || amount(str) || nonce(8)`
    ///
    /// This canonical form ensures deterministic verification across nodes.
    #[must_use]
    pub fn signing_payload(&self) -> Vec<u8> {
        let mut payload = Vec::with_capacity(128);
        payload.extend_from_slice(self.order_id.0.as_bytes());
        payload.extend_from_slice(self.user_id.0.as_bytes());
        payload.extend_from_slice(self.asset.as_bytes());
        payload.extend_from_slice(self.amount.to_string().as_bytes());
        payload.extend_from_slice(&self.nonce.to_le_bytes());
        payload
    }

    /// Returns `true` if this proof has expired.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    /// Returns `true` if this proof will expire within the given duration.
    #[must_use]
    pub fn expires_within(&self, duration: chrono::Duration) -> bool {
        Utc::now() + duration > self.expires_at
    }

    /// Returns the duration until expiry, or zero if already expired.
    #[must_use]
    pub fn time_until_expiry(&self) -> chrono::Duration {
        let now = Utc::now();
        if now > self.expires_at {
            chrono::Duration::zero()
        } else {
            self.expires_at - now
        }
    }
}

/// A placeholder / dummy freeze proof for testing.
/// **Never use in production.**
#[cfg(any(test, feature = "test-helpers"))]
impl FreezeProof {
    /// Create a dummy proof for unit tests. Signature is empty.
    pub fn dummy(order_id: OrderId, user_id: UserId, asset: &str, amount: Decimal) -> Self {
        Self {
            order_id,
            user_id,
            asset: asset.to_string(),
            amount,
            issuer_node: NodeId([0u8; 32]),
            signature: vec![0u8; 64],
            nonce: 0,
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_proof() -> FreezeProof {
        FreezeProof::dummy(
            OrderId::new(),
            UserId::new(),
            "USDT",
            Decimal::new(10000, 2), // 100.00
        )
    }

    #[test]
    fn signing_payload_deterministic() {
        let proof = make_proof();
        let a = proof.signing_payload();
        let b = proof.signing_payload();
        assert_eq!(a, b, "Same proof must produce same payload");
    }

    #[test]
    fn signing_payload_differs_by_nonce() {
        let mut p1 = make_proof();
        p1.nonce = 1;
        let mut p2 = p1.clone();
        p2.nonce = 2;
        assert_ne!(
            p1.signing_payload(),
            p2.signing_payload(),
            "Different nonce must produce different payload"
        );
    }

    #[test]
    fn is_expired_future() {
        let proof = make_proof();
        assert!(!proof.is_expired(), "Proof expiring in 1 hour should not be expired");
    }

    #[test]
    fn is_expired_past() {
        let mut proof = make_proof();
        proof.expires_at = Utc::now() - chrono::Duration::seconds(1);
        assert!(proof.is_expired(), "Proof in the past should be expired");
    }

    #[test]
    fn serde_roundtrip() {
        let proof = make_proof();
        let json = serde_json::to_string(&proof).unwrap();
        let back: FreezeProof = serde_json::from_str(&json).unwrap();
        assert_eq!(proof.order_id, back.order_id);
        assert_eq!(proof.asset, back.asset);
        assert_eq!(proof.amount, back.amount);
    }
}
