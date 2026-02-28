//! Batch sealer — produces `SealedBatch` and `BatchDigest`.
//!
//! When the SEAL phase begins, the `BatchSealer` takes the contents
//! of the `PendingBuffer`, sorts them deterministically, computes
//! the batch hash, and produces the immutable `SealedBatch`.

use chrono::Utc;
use openmatch_types::{BatchDigest, EpochId, NodeId, Order, SealedBatch};
use sha2::{Digest, Sha256};

/// Seals pending orders into an immutable `SealedBatch`.
pub struct BatchSealer {
    /// The node identity for signing digests.
    node_id: NodeId,
}

impl BatchSealer {
    /// Create a new batch sealer for the given node.
    #[must_use]
    pub fn new(node_id: NodeId) -> Self {
        Self { node_id }
    }

    /// Seal a set of orders into a `SealedBatch`.
    ///
    /// 1. Sort orders deterministically by sequence number
    /// 2. Compute the batch hash (SHA-256 over all order data)
    /// 3. Return the sealed batch
    #[must_use]
    pub fn seal(&self, epoch_id: EpochId, mut orders: Vec<Order>) -> SealedBatch {
        // Deterministic sort: by sequence, then by order ID for tie-breaking
        orders.sort_by(|a, b| a.sequence.cmp(&b.sequence).then(a.id.cmp(&b.id)));

        // Compute batch hash
        let batch_hash = Self::compute_batch_hash(epoch_id, &orders);

        SealedBatch {
            epoch_id,
            orders,
            batch_hash,
            sealed_at: Utc::now(),
            sealer_node: self.node_id,
        }
    }

    /// Create a `BatchDigest` from a `SealedBatch` for gossip exchange.
    ///
    /// The digest contains only metadata — not the full order set.
    /// Nodes compare digests to verify they sealed the same batch.
    #[must_use]
    pub fn digest(&self, batch: &SealedBatch) -> BatchDigest {
        BatchDigest {
            epoch_id: batch.epoch_id,
            batch_hash: batch.batch_hash,
            order_count: batch.orders.len(),
            signer_node: self.node_id,
            // Signature would be computed with the node's ed25519 key.
            // For now, placeholder.
            signature: vec![0u8; 64],
        }
    }

    /// Compute the SHA-256 hash over the ordered set of orders.
    ///
    /// This hash commits to:
    /// - Epoch ID
    /// - Number of orders
    /// - Each order's ID, user_id, side, type, price, quantity, sequence
    fn compute_batch_hash(epoch_id: EpochId, orders: &[Order]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"openmatch:batch:v2:");
        hasher.update(epoch_id.0.to_le_bytes());
        hasher.update((orders.len() as u64).to_le_bytes());

        for order in orders {
            hasher.update(order.id.0.as_bytes());
            hasher.update(order.user_id.0.as_bytes());
            hasher.update(order.sr_id.0.as_bytes());
            hasher.update(match order.side {
                openmatch_types::OrderSide::Buy => &[0u8],
                openmatch_types::OrderSide::Sell => &[1u8],
            });
            hasher.update(match order.order_type {
                openmatch_types::OrderType::Limit => &[0u8],
                openmatch_types::OrderType::Market => &[1u8],
                openmatch_types::OrderType::Cancel => &[2u8],
            });
            if let Some(price) = &order.price {
                hasher.update(price.to_string().as_bytes());
            }
            hasher.update(order.quantity.to_string().as_bytes());
            hasher.update(order.sequence.to_le_bytes());
        }

        let result = hasher.finalize();
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&result);
        hash
    }

    /// Verify that two batch hashes match.
    #[must_use]
    pub fn verify_batch_hash(batch: &SealedBatch) -> bool {
        let expected = Self::compute_batch_hash(batch.epoch_id, &batch.orders);
        expected == batch.batch_hash
    }
}

#[cfg(test)]
mod tests {
    use openmatch_types::*;
    use rust_decimal::Decimal;

    use super::*;

    fn make_sealer() -> BatchSealer {
        BatchSealer::new(NodeId([0u8; 32]))
    }

    #[test]
    fn seal_empty_batch() {
        let sealer = make_sealer();
        let batch = sealer.seal(EpochId(1), vec![]);
        assert!(batch.orders.is_empty());
        assert_eq!(batch.epoch_id, EpochId(1));
        assert_ne!(batch.batch_hash, [0u8; 32]); // Hash should not be zero
    }

    #[test]
    fn seal_sorts_by_sequence() {
        let sealer = make_sealer();
        let mut o1 = Order::dummy_limit(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE);
        o1.sequence = 2;
        let mut o2 = Order::dummy_limit(OrderSide::Sell, Decimal::new(101, 0), Decimal::ONE);
        o2.sequence = 0;
        let mut o3 = Order::dummy_limit(OrderSide::Buy, Decimal::new(99, 0), Decimal::ONE);
        o3.sequence = 1;

        let batch = sealer.seal(EpochId(1), vec![o1, o2, o3]);

        assert_eq!(batch.orders[0].sequence, 0);
        assert_eq!(batch.orders[1].sequence, 1);
        assert_eq!(batch.orders[2].sequence, 2);
    }

    #[test]
    fn batch_hash_is_deterministic() {
        let sealer = make_sealer();
        let orders = vec![
            Order::dummy_limit(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE),
            Order::dummy_limit(OrderSide::Sell, Decimal::new(101, 0), Decimal::ONE),
        ];

        let batch1 = sealer.seal(EpochId(1), orders.clone());
        let batch2 = sealer.seal(EpochId(1), orders);

        assert_eq!(batch1.batch_hash, batch2.batch_hash);
    }

    #[test]
    fn different_epochs_different_hash() {
        let sealer = make_sealer();
        let orders = vec![Order::dummy_limit(
            OrderSide::Buy,
            Decimal::new(100, 0),
            Decimal::ONE,
        )];

        let batch1 = sealer.seal(EpochId(1), orders.clone());
        let batch2 = sealer.seal(EpochId(2), orders);

        assert_ne!(batch1.batch_hash, batch2.batch_hash);
    }

    #[test]
    fn verify_batch_hash_passes() {
        let sealer = make_sealer();
        let orders = vec![Order::dummy_limit(
            OrderSide::Buy,
            Decimal::new(100, 0),
            Decimal::ONE,
        )];
        let batch = sealer.seal(EpochId(1), orders);
        assert!(BatchSealer::verify_batch_hash(&batch));
    }

    #[test]
    fn tampered_batch_hash_fails() {
        let sealer = make_sealer();
        let orders = vec![Order::dummy_limit(
            OrderSide::Buy,
            Decimal::new(100, 0),
            Decimal::ONE,
        )];
        let mut batch = sealer.seal(EpochId(1), orders);
        batch.batch_hash[0] ^= 0xFF; // Tamper
        assert!(!BatchSealer::verify_batch_hash(&batch));
    }

    #[test]
    fn digest_matches_batch() {
        let sealer = make_sealer();
        let orders = vec![
            Order::dummy_limit(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE),
            Order::dummy_limit(OrderSide::Sell, Decimal::new(101, 0), Decimal::ONE),
        ];
        let batch = sealer.seal(EpochId(1), orders);
        let digest = sealer.digest(&batch);

        assert_eq!(digest.epoch_id, batch.epoch_id);
        assert_eq!(digest.batch_hash, batch.batch_hash);
        assert_eq!(digest.order_count, 2);
    }
}
