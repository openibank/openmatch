//! Settlement idempotency guard — prevents double-settlement.
//!
//! Like blockchain UTXO: each trade can only be settled once. Attempting to
//! settle the same `TradeId` a second time returns
//! [`OpenmatchError::TradeAlreadySettled`].
//!
//! The guard maintains an LRU-style bounded cache so memory usage stays
//! predictable in long-running nodes.

use std::collections::{HashSet, VecDeque};

use openmatch_types::{OpenmatchError, Result, TradeId};

/// Prevents double-settlement of the same trade.
///
/// Internally stores a bounded set of settled `TradeId`s with LRU eviction.
/// When the set reaches `max_size`, the oldest entry is evicted to make room.
pub struct IdempotencyGuard {
    /// Set of trade IDs that have already been settled.
    settled: HashSet<TradeId>,
    /// Insertion order for LRU eviction (front = oldest).
    order: VecDeque<TradeId>,
    /// Maximum number of entries before eviction kicks in.
    max_size: usize,
}

impl IdempotencyGuard {
    /// Create a new guard with the given maximum cache size.
    ///
    /// # Panics
    /// Panics if `max_size` is zero.
    pub fn new(max_size: usize) -> Self {
        assert!(max_size > 0, "IdempotencyGuard max_size must be > 0");
        Self {
            settled: HashSet::with_capacity(max_size),
            order: VecDeque::with_capacity(max_size),
            max_size,
        }
    }

    /// Mark a trade as settled. Returns an error if the trade was already
    /// settled (idempotency violation).
    ///
    /// # Errors
    /// Returns [`OpenmatchError::TradeAlreadySettled`] if `trade_id` has
    /// already been marked as settled.
    pub fn mark_settled(&mut self, trade_id: TradeId) -> Result<()> {
        if self.settled.contains(&trade_id) {
            return Err(OpenmatchError::TradeAlreadySettled(trade_id));
        }

        // Evict oldest if at capacity.
        if self.settled.len() >= self.max_size {
            if let Some(oldest) = self.order.pop_front() {
                self.settled.remove(&oldest);
            }
        }

        self.settled.insert(trade_id);
        self.order.push_back(trade_id);
        Ok(())
    }

    /// Check whether a trade has already been settled.
    pub fn is_settled(&self, trade_id: &TradeId) -> bool {
        self.settled.contains(trade_id)
    }

    /// Number of trades currently tracked.
    pub fn len(&self) -> usize {
        self.settled.len()
    }

    /// Whether the guard is empty (no trades tracked).
    pub fn is_empty(&self) -> bool {
        self.settled.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_settle_ok() {
        let mut guard = IdempotencyGuard::new(100);
        let trade_id = TradeId::new();
        assert!(guard.mark_settled(trade_id).is_ok());
        assert!(guard.is_settled(&trade_id));
        assert_eq!(guard.len(), 1);
    }

    #[test]
    fn double_settle_blocked() {
        let mut guard = IdempotencyGuard::new(100);
        let trade_id = TradeId::new();
        guard.mark_settled(trade_id).unwrap();

        let err = guard.mark_settled(trade_id).unwrap_err();
        assert!(
            matches!(err, OpenmatchError::TradeAlreadySettled(id) if id == trade_id),
            "Expected TradeAlreadySettled, got: {err:?}"
        );
    }

    #[test]
    fn evicts_oldest() {
        let mut guard = IdempotencyGuard::new(3);
        let t1 = TradeId::deterministic(1, 0);
        let t2 = TradeId::deterministic(1, 1);
        let t3 = TradeId::deterministic(1, 2);
        let t4 = TradeId::deterministic(1, 3);

        guard.mark_settled(t1).unwrap();
        guard.mark_settled(t2).unwrap();
        guard.mark_settled(t3).unwrap();
        assert_eq!(guard.len(), 3);

        // Adding t4 should evict t1 (the oldest).
        guard.mark_settled(t4).unwrap();
        assert_eq!(guard.len(), 3);
        assert!(!guard.is_settled(&t1), "t1 should have been evicted");
        assert!(guard.is_settled(&t2));
        assert!(guard.is_settled(&t3));
        assert!(guard.is_settled(&t4));
    }

    #[test]
    fn different_trades_ok() {
        let mut guard = IdempotencyGuard::new(100);
        let t1 = TradeId::deterministic(1, 0);
        let t2 = TradeId::deterministic(1, 1);
        let t3 = TradeId::deterministic(2, 0);

        guard.mark_settled(t1).unwrap();
        guard.mark_settled(t2).unwrap();
        guard.mark_settled(t3).unwrap();

        assert_eq!(guard.len(), 3);
        assert!(guard.is_settled(&t1));
        assert!(guard.is_settled(&t2));
        assert!(guard.is_settled(&t3));
    }

    #[test]
    fn empty_guard() {
        let guard = IdempotencyGuard::new(10);
        assert!(guard.is_empty());
        assert_eq!(guard.len(), 0);
        assert!(!guard.is_settled(&TradeId::new()));
    }

    #[test]
    #[should_panic(expected = "max_size must be > 0")]
    fn zero_max_size_panics() {
        let _ = IdempotencyGuard::new(0);
    }
}
