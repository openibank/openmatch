//! A single price level in the order book.
//!
//! Orders at the same price are stored in FIFO order (time priority)
//! using a [`VecDeque`].

use std::collections::VecDeque;

use openmatch_types::{Order, OrderId};
use rust_decimal::Decimal;

/// A single price level containing all orders at that price.
///
/// Orders are stored in arrival order (FIFO) -- the front of the deque
/// has the highest time priority and will be filled first.
#[derive(Debug, Clone)]
pub struct PriceLevel {
    /// The price at this level.
    pub price: Decimal,
    /// Orders in time-priority order (front = oldest = highest priority).
    pub orders: VecDeque<Order>,
}

impl PriceLevel {
    /// Create a new empty price level.
    #[must_use]
    pub fn new(price: Decimal) -> Self {
        Self {
            price,
            orders: VecDeque::new(),
        }
    }

    /// Add an order to the back of this level (lowest time priority).
    pub fn push_back(&mut self, order: Order) {
        self.orders.push_back(order);
    }

    /// Remove and return the front (oldest / highest priority) order.
    pub fn pop_front(&mut self) -> Option<Order> {
        self.orders.pop_front()
    }

    /// Peek at the front order without removing it.
    #[must_use]
    pub fn front(&self) -> Option<&Order> {
        self.orders.front()
    }

    /// Total remaining quantity across all orders at this level.
    #[must_use]
    pub fn total_quantity(&self) -> Decimal {
        self.orders.iter().map(|o| o.remaining_qty).sum()
    }

    /// Remove a specific order by ID. Returns the removed order, or `None`.
    pub fn remove_order(&mut self, order_id: &OrderId) -> Option<Order> {
        let pos = self.orders.iter().position(|o| o.id == *order_id)?;
        self.orders.remove(pos)
    }

    /// Returns `true` if there are no orders at this level.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }

    /// Number of orders at this level.
    #[must_use]
    pub fn len(&self) -> usize {
        self.orders.len()
    }
}

#[cfg(test)]
mod tests {
    use openmatch_types::*;
    use rust_decimal::Decimal;

    use super::*;

    fn make_order(price: Decimal, qty: Decimal, seq: u64) -> Order {
        let mut order = Order::dummy_limit(OrderSide::Buy, price, qty);
        order.sequence = seq;
        order
    }

    #[test]
    fn push_pop_fifo() {
        let mut level = PriceLevel::new(Decimal::new(100, 0));
        let o1 = make_order(Decimal::new(100, 0), Decimal::ONE, 0);
        let o2 = make_order(Decimal::new(100, 0), Decimal::ONE, 1);
        let id1 = o1.id;

        level.push_back(o1);
        level.push_back(o2);

        assert_eq!(level.len(), 2);
        let popped = level.pop_front().unwrap();
        assert_eq!(popped.id, id1, "FIFO: first in should be first out");
        assert_eq!(level.len(), 1);
    }

    #[test]
    fn total_quantity() {
        let mut level = PriceLevel::new(Decimal::new(100, 0));
        level.push_back(make_order(Decimal::new(100, 0), Decimal::new(5, 0), 0));
        level.push_back(make_order(Decimal::new(100, 0), Decimal::new(3, 0), 1));
        assert_eq!(level.total_quantity(), Decimal::new(8, 0));
    }

    #[test]
    fn remove_order_by_id() {
        let mut level = PriceLevel::new(Decimal::new(100, 0));
        let o1 = make_order(Decimal::new(100, 0), Decimal::ONE, 0);
        let o2 = make_order(Decimal::new(100, 0), Decimal::ONE, 1);
        let target_id = o2.id;

        level.push_back(o1);
        level.push_back(o2);

        let removed = level.remove_order(&target_id);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().id, target_id);
        assert_eq!(level.len(), 1);
    }

    #[test]
    fn remove_nonexistent_order() {
        let mut level = PriceLevel::new(Decimal::new(100, 0));
        level.push_back(make_order(Decimal::new(100, 0), Decimal::ONE, 0));
        let fake_id = OrderId::new();
        assert!(level.remove_order(&fake_id).is_none());
    }

    #[test]
    fn empty_level() {
        let level = PriceLevel::new(Decimal::new(100, 0));
        assert!(level.is_empty());
        assert_eq!(level.len(), 0);
        assert_eq!(level.total_quantity(), Decimal::ZERO);
        assert!(level.front().is_none());
    }
}
