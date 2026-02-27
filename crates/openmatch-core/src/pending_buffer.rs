//! Pending buffer for the COLLECT phase of an epoch.
//!
//! Orders flow into the [`PendingBuffer`] during the COLLECT phase.
//! When the epoch transitions to MATCH, the buffer is **sealed**:
//! - Orders are sorted deterministically
//! - A SHA-256 batch hash is computed over the canonical representation
//! - No more orders can be added
//!
//! The sealed buffer is then consumed by the [`BatchMatcher`](crate::BatchMatcher).

use openmatch_types::*;
use sha2::{Digest, Sha256};

/// Collects orders during the COLLECT phase and seals them for matching.
#[derive(Debug)]
pub struct PendingBuffer {
    /// Orders collected so far.
    orders: Vec<Order>,
    /// Monotonic sequence counter for time-priority ordering.
    sequence_counter: u64,
    /// Whether the buffer has been sealed.
    sealed: bool,
    /// SHA-256 hash computed at seal time.
    batch_hash: Option<[u8; 32]>,
    /// The batch this buffer belongs to.
    batch_id: BatchId,
}

impl PendingBuffer {
    /// Create a new empty buffer for the given batch.
    #[must_use]
    pub fn new(batch_id: BatchId) -> Self {
        Self {
            orders: Vec::new(),
            sequence_counter: 0,
            sealed: false,
            batch_hash: None,
            batch_id,
        }
    }

    /// Add an order to the buffer. Assigns a monotonic sequence number.
    ///
    /// # Errors
    /// Returns `BufferAlreadySealed` if the buffer has been sealed.
    /// Returns `BufferFull` if `MAX_ORDERS_PER_BATCH` is reached.
    pub fn push(&mut self, mut order: Order) -> Result<u64> {
        if self.sealed {
            return Err(OpenmatchError::BufferAlreadySealed);
        }
        if self.orders.len() >= constants::MAX_ORDERS_PER_BATCH {
            return Err(OpenmatchError::BufferFull);
        }

        let seq = self.sequence_counter;
        order.sequence = seq;
        order.batch_id = Some(self.batch_id);
        self.sequence_counter += 1;
        self.orders.push(order);
        Ok(seq)
    }

    /// Seal the buffer: sort orders deterministically, compute `batch_hash`.
    ///
    /// **Sort order:**
    /// 1. Side: Buy before Sell (enum ordinal)
    /// 2. Price priority: Buy = highest first, Sell = lowest first
    /// 3. Sequence: lowest first (time priority)
    ///
    /// This ensures determinism: same set of orders → same sorted order → same hash.
    ///
    /// # Errors
    /// Returns `BufferAlreadySealed` if already sealed.
    pub fn seal(&mut self) -> Result<[u8; 32]> {
        if self.sealed {
            return Err(OpenmatchError::BufferAlreadySealed);
        }

        // Deterministic sort
        self.orders.sort_by(|a, b| {
            a.side
                .cmp(&b.side) // Buy < Sell
                .then_with(|| match a.side {
                    // Buys: higher price first (descending)
                    OrderSide::Buy => b.effective_price().cmp(&a.effective_price()),
                    // Sells: lower price first (ascending)
                    OrderSide::Sell => a.effective_price().cmp(&b.effective_price()),
                })
                .then_with(|| a.sequence.cmp(&b.sequence)) // time priority
        });

        // Compute SHA-256 hash over canonical representation
        let mut hasher = Sha256::new();
        hasher.update(b"openmatch:batch:v1:");
        hasher.update(self.batch_id.0.to_le_bytes());
        hasher.update((self.orders.len() as u64).to_le_bytes());
        for order in &self.orders {
            hasher.update(order.id.0.as_bytes());
            hasher.update(order.sequence.to_le_bytes());
            hasher.update(order.effective_price().to_string().as_bytes());
            hasher.update(order.remaining_qty.to_string().as_bytes());
            match order.side {
                OrderSide::Buy => hasher.update([0u8]),
                OrderSide::Sell => hasher.update([1u8]),
            }
        }
        let hash: [u8; 32] = hasher.finalize().into();

        self.sealed = true;
        self.batch_hash = Some(hash);
        Ok(hash)
    }

    /// Consume the sealed buffer, returning the sorted orders and batch hash.
    ///
    /// # Errors
    /// Returns `MatchingFailed` if not sealed.
    pub fn take_orders(self) -> Result<(Vec<Order>, [u8; 32])> {
        if !self.sealed {
            return Err(OpenmatchError::MatchingFailed {
                reason: "Cannot take orders from unsealed buffer".into(),
            });
        }
        Ok((self.orders, self.batch_hash.unwrap()))
    }

    /// Whether the buffer has been sealed.
    #[must_use]
    pub fn is_sealed(&self) -> bool {
        self.sealed
    }

    /// The batch hash (only available after sealing).
    #[must_use]
    pub fn batch_hash(&self) -> Option<[u8; 32]> {
        self.batch_hash
    }

    /// Number of orders in the buffer.
    #[must_use]
    pub fn len(&self) -> usize {
        self.orders.len()
    }

    /// Whether the buffer is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }

    /// The batch ID of this buffer.
    #[must_use]
    pub fn batch_id(&self) -> BatchId {
        self.batch_id
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use openmatch_types::*;
    use rust_decimal::Decimal;

    use super::*;

    fn make_order(side: OrderSide, price: Decimal, qty: Decimal) -> Order {
        let id = OrderId::new();
        let user_id = UserId::new();
        let asset = match side {
            OrderSide::Buy => "USDT",
            OrderSide::Sell => "BTC",
        };
        Order {
            id,
            user_id,
            market: MarketPair::new("BTC", "USDT"),
            side,
            order_type: OrderType::Limit,
            status: OrderStatus::Active,
            price: Some(price),
            quantity: qty,
            remaining_qty: qty,
            freeze_proof: FreezeProof::dummy(id, user_id, asset, price * qty),
            batch_id: None,
            origin_node: NodeId([0u8; 32]),
            sequence: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn push_assigns_sequence() {
        let mut buf = PendingBuffer::new(BatchId(1));
        let s0 = buf
            .push(make_order(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE))
            .unwrap();
        let s1 = buf
            .push(make_order(OrderSide::Sell, Decimal::new(101, 0), Decimal::ONE))
            .unwrap();
        assert_eq!(s0, 0);
        assert_eq!(s1, 1);
        assert_eq!(buf.len(), 2);
    }

    #[test]
    fn push_after_seal_fails() {
        let mut buf = PendingBuffer::new(BatchId(1));
        buf.push(make_order(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE))
            .unwrap();
        buf.seal().unwrap();

        let result = buf.push(make_order(
            OrderSide::Buy,
            Decimal::new(100, 0),
            Decimal::ONE,
        ));
        assert!(matches!(result, Err(OpenmatchError::BufferAlreadySealed)));
    }

    #[test]
    fn double_seal_fails() {
        let mut buf = PendingBuffer::new(BatchId(1));
        buf.push(make_order(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE))
            .unwrap();
        buf.seal().unwrap();
        assert!(matches!(buf.seal(), Err(OpenmatchError::BufferAlreadySealed)));
    }

    #[test]
    fn seal_sorts_deterministically() {
        // Create two buffers with same orders in different insertion order
        let orders = vec![
            make_order(OrderSide::Sell, Decimal::new(105, 0), Decimal::ONE),
            make_order(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE),
            make_order(OrderSide::Buy, Decimal::new(102, 0), Decimal::ONE),
            make_order(OrderSide::Sell, Decimal::new(103, 0), Decimal::ONE),
        ];

        let mut buf1 = PendingBuffer::new(BatchId(1));
        let mut buf2 = PendingBuffer::new(BatchId(1));

        // Insert in original order
        for o in &orders {
            buf1.push(o.clone()).unwrap();
        }

        // Insert in reverse order
        for o in orders.iter().rev() {
            buf2.push(o.clone()).unwrap();
        }

        let _hash1 = buf1.seal().unwrap();
        let _hash2 = buf2.seal().unwrap();

        // Same orders (by value) in same batch → same hash
        // Note: these have DIFFERENT OrderIds so hashes will differ.
        // But the sort ORDER should be deterministic:
        // Both should sort as: Buy@102, Buy@100, Sell@103, Sell@105
        let (orders1, _) = buf1.take_orders().unwrap();
        let (orders2, _) = buf2.take_orders().unwrap();

        // Verify sort order (buys desc price, sells asc price)
        assert_eq!(orders1[0].side, OrderSide::Buy);
        assert_eq!(orders1[0].effective_price(), Decimal::new(102, 0));
        assert_eq!(orders1[1].side, OrderSide::Buy);
        assert_eq!(orders1[1].effective_price(), Decimal::new(100, 0));
        assert_eq!(orders1[2].side, OrderSide::Sell);
        assert_eq!(orders1[2].effective_price(), Decimal::new(103, 0));
        assert_eq!(orders1[3].side, OrderSide::Sell);
        assert_eq!(orders1[3].effective_price(), Decimal::new(105, 0));

        // Same sort order in both buffers
        for i in 0..4 {
            assert_eq!(orders1[i].side, orders2[i].side);
            assert_eq!(orders1[i].effective_price(), orders2[i].effective_price());
        }
    }

    #[test]
    fn seal_produces_consistent_hash() {
        // Same buffer sealed twice (via clone before seal) → same hash
        let mut buf = PendingBuffer::new(BatchId(42));
        let o1 = make_order(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE);
        let o2 = make_order(OrderSide::Sell, Decimal::new(101, 0), Decimal::ONE);
        buf.push(o1.clone()).unwrap();
        buf.push(o2.clone()).unwrap();

        // Clone the state before sealing
        let orders_snapshot: Vec<Order> = vec![o1, o2];

        let hash1 = buf.seal().unwrap();

        // Rebuild with exact same orders
        let mut buf2 = PendingBuffer::new(BatchId(42));
        for o in orders_snapshot {
            buf2.push(o).unwrap();
        }
        let hash2 = buf2.seal().unwrap();

        assert_eq!(hash1, hash2, "Same orders in same batch → same hash");
    }

    #[test]
    fn take_orders_before_seal_fails() {
        let buf = PendingBuffer::new(BatchId(1));
        assert!(buf.take_orders().is_err());
    }

    #[test]
    fn empty_buffer() {
        let buf = PendingBuffer::new(BatchId(1));
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
        assert!(!buf.is_sealed());
        assert_eq!(buf.batch_hash(), None);
    }

    #[test]
    fn empty_buffer_can_seal() {
        let mut buf = PendingBuffer::new(BatchId(1));
        let hash = buf.seal().unwrap();
        assert!(buf.is_sealed());
        assert_eq!(buf.batch_hash(), Some(hash));
        let (orders, _) = buf.take_orders().unwrap();
        assert!(orders.is_empty());
    }
}
