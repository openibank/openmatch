//! Pending buffer for order collection during the COLLECT phase.
//!
//! Orders that have passed risk validation and have an active SpendRight
//! are pushed into the PendingBuffer. When the SEAL phase begins, the
//! buffer is sealed into a `SealedBatch`.

use openmatch_types::{constants, OpenmatchError, Order, Result};

/// Collects validated orders during the COLLECT phase.
///
/// Once sealed, no more orders can be added. The buffer is consumed
/// by the `BatchSealer` to produce a `SealedBatch`.
pub struct PendingBuffer {
    /// Orders in arrival order.
    orders: Vec<Order>,
    /// Whether the buffer has been sealed.
    sealed: bool,
    /// Maximum number of orders before the buffer is full.
    max_orders: usize,
}

impl PendingBuffer {
    /// Create a new empty buffer with the default max size.
    #[must_use]
    pub fn new() -> Self {
        Self {
            orders: Vec::new(),
            sealed: false,
            max_orders: constants::MAX_ORDERS_PER_BATCH,
        }
    }

    /// Create a buffer with a custom max size.
    #[must_use]
    pub fn with_capacity(max_orders: usize) -> Self {
        Self {
            orders: Vec::with_capacity(max_orders),
            sealed: false,
            max_orders,
        }
    }

    /// Push a validated order into the buffer.
    ///
    /// # Errors
    /// - `BufferAlreadySealed` if the buffer has been sealed
    /// - `BufferFull` if the buffer is at capacity
    pub fn push(&mut self, order: Order) -> Result<()> {
        if self.sealed {
            return Err(OpenmatchError::BufferAlreadySealed);
        }
        if self.orders.len() >= self.max_orders {
            return Err(OpenmatchError::BufferFull);
        }
        self.orders.push(order);
        Ok(())
    }

    /// Seal the buffer. No more orders can be added after this.
    ///
    /// # Errors
    /// Returns `BufferAlreadySealed` if already sealed.
    pub fn seal(&mut self) -> Result<()> {
        if self.sealed {
            return Err(OpenmatchError::BufferAlreadySealed);
        }
        self.sealed = true;
        Ok(())
    }

    /// Drain all orders from the buffer (consumes the content).
    ///
    /// Used by the `BatchSealer` to extract orders for the `SealedBatch`.
    /// The buffer must be sealed before draining.
    ///
    /// # Errors
    /// Returns error if the buffer is not sealed.
    pub fn drain(&mut self) -> Result<Vec<Order>> {
        if !self.sealed {
            return Err(OpenmatchError::InvalidOrder {
                reason: "Cannot drain unsealed buffer".to_string(),
            });
        }
        Ok(std::mem::take(&mut self.orders))
    }

    /// Whether the buffer has been sealed.
    #[must_use]
    pub fn is_sealed(&self) -> bool {
        self.sealed
    }

    /// Number of orders currently in the buffer.
    #[must_use]
    pub fn len(&self) -> usize {
        self.orders.len()
    }

    /// Whether the buffer is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }

    /// Reset the buffer for a new epoch.
    pub fn reset(&mut self) {
        self.orders.clear();
        self.sealed = false;
    }
}

impl Default for PendingBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openmatch_types::*;
    use rust_decimal::Decimal;

    #[test]
    fn push_and_count() {
        let mut buf = PendingBuffer::new();
        buf.push(Order::dummy_limit(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE))
            .unwrap();
        buf.push(Order::dummy_limit(OrderSide::Sell, Decimal::new(101, 0), Decimal::ONE))
            .unwrap();
        assert_eq!(buf.len(), 2);
        assert!(!buf.is_empty());
    }

    #[test]
    fn push_after_seal_fails() {
        let mut buf = PendingBuffer::new();
        buf.seal().unwrap();
        let err = buf
            .push(Order::dummy_limit(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE))
            .unwrap_err();
        assert!(matches!(err, OpenmatchError::BufferAlreadySealed));
    }

    #[test]
    fn double_seal_fails() {
        let mut buf = PendingBuffer::new();
        buf.seal().unwrap();
        let err = buf.seal().unwrap_err();
        assert!(matches!(err, OpenmatchError::BufferAlreadySealed));
    }

    #[test]
    fn buffer_full() {
        let mut buf = PendingBuffer::with_capacity(2);
        buf.push(Order::dummy_limit(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE))
            .unwrap();
        buf.push(Order::dummy_limit(OrderSide::Sell, Decimal::new(101, 0), Decimal::ONE))
            .unwrap();
        let err = buf
            .push(Order::dummy_limit(OrderSide::Buy, Decimal::new(99, 0), Decimal::ONE))
            .unwrap_err();
        assert!(matches!(err, OpenmatchError::BufferFull));
    }

    #[test]
    fn drain_returns_all_orders() {
        let mut buf = PendingBuffer::new();
        buf.push(Order::dummy_limit(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE))
            .unwrap();
        buf.push(Order::dummy_limit(OrderSide::Sell, Decimal::new(101, 0), Decimal::ONE))
            .unwrap();
        buf.seal().unwrap();
        let orders = buf.drain().unwrap();
        assert_eq!(orders.len(), 2);
        assert!(buf.is_empty());
    }

    #[test]
    fn drain_unsealed_fails() {
        let mut buf = PendingBuffer::new();
        buf.push(Order::dummy_limit(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE))
            .unwrap();
        assert!(buf.drain().is_err());
    }

    #[test]
    fn reset_clears_everything() {
        let mut buf = PendingBuffer::new();
        buf.push(Order::dummy_limit(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE))
            .unwrap();
        buf.seal().unwrap();
        buf.reset();
        assert!(buf.is_empty());
        assert!(!buf.is_sealed());
        // Should be able to push again
        buf.push(Order::dummy_limit(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE))
            .unwrap();
    }
}
