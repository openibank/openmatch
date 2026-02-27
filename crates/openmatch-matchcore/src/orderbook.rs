//! The order book for a single market pair.
//!
//! Uses `BTreeMap` for price-level ordering:
//! - **Bids** (buys): `BTreeMap<Reverse<Decimal>, PriceLevel>` -- highest price first
//! - **Asks** (sells): `BTreeMap<Decimal, PriceLevel>` -- lowest price first
//!
//! An auxiliary `HashMap<OrderId, (Side, Price)>` enables O(log N) cancellation.

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap};

use openmatch_types::*;
use rust_decimal::Decimal;

use crate::price_level::PriceLevel;

/// The order book for a single market pair.
#[derive(Debug)]
pub struct OrderBook {
    /// The market this book serves (e.g., BTC/USDT).
    pub market: MarketPair,
    /// Buy side: highest price first (`Reverse` key).
    bids: BTreeMap<Reverse<Decimal>, PriceLevel>,
    /// Sell side: lowest price first.
    asks: BTreeMap<Decimal, PriceLevel>,
    /// Fast lookup: `OrderId -> (side, price)` for O(log N) cancel.
    index: HashMap<OrderId, (OrderSide, Decimal)>,
}

impl OrderBook {
    /// Create a new empty order book for the given market.
    #[must_use]
    pub fn new(market: MarketPair) -> Self {
        Self {
            market,
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            index: HashMap::new(),
        }
    }

    // =================================================================
    // Insertion
    // =================================================================

    /// Insert a single order into the book at its effective price.
    pub fn insert_order(&mut self, order: Order) -> Result<()> {
        if self.index.contains_key(&order.id) {
            return Err(OpenmatchError::DuplicateOrder(order.id));
        }

        let price = order.effective_price();
        self.index.insert(order.id, (order.side, price));

        match order.side {
            OrderSide::Buy => {
                self.bids
                    .entry(Reverse(price))
                    .or_insert_with(|| PriceLevel::new(price))
                    .push_back(order);
            }
            OrderSide::Sell => {
                self.asks
                    .entry(price)
                    .or_insert_with(|| PriceLevel::new(price))
                    .push_back(order);
            }
        }
        Ok(())
    }

    /// Insert a batch of orders from a sealed pending buffer.
    pub fn insert_batch(&mut self, orders: Vec<Order>) -> Result<()> {
        for order in orders {
            self.insert_order(order)?;
        }
        Ok(())
    }

    // =================================================================
    // Cancellation
    // =================================================================

    /// Cancel an order by ID. Returns the removed order.
    pub fn cancel_order(&mut self, order_id: &OrderId) -> Result<Order> {
        let (side, price) = self
            .index
            .remove(order_id)
            .ok_or(OpenmatchError::OrderNotFound(*order_id))?;

        let order = match side {
            OrderSide::Buy => {
                let level = self
                    .bids
                    .get_mut(&Reverse(price))
                    .ok_or(OpenmatchError::OrderNotFound(*order_id))?;
                let order = level
                    .remove_order(order_id)
                    .ok_or(OpenmatchError::OrderNotFound(*order_id))?;
                if level.is_empty() {
                    self.bids.remove(&Reverse(price));
                }
                order
            }
            OrderSide::Sell => {
                let level = self
                    .asks
                    .get_mut(&price)
                    .ok_or(OpenmatchError::OrderNotFound(*order_id))?;
                let order = level
                    .remove_order(order_id)
                    .ok_or(OpenmatchError::OrderNotFound(*order_id))?;
                if level.is_empty() {
                    self.asks.remove(&price);
                }
                order
            }
        };

        Ok(order)
    }

    // =================================================================
    // Queries
    // =================================================================

    /// Best (highest) bid price, or `None` if no bids.
    #[must_use]
    pub fn best_bid(&self) -> Option<Decimal> {
        self.bids.keys().next().map(|r| r.0)
    }

    /// Best (lowest) ask price, or `None` if no asks.
    #[must_use]
    pub fn best_ask(&self) -> Option<Decimal> {
        self.asks.keys().next().copied()
    }

    /// Spread = best_ask - best_bid. `None` if either side is empty.
    #[must_use]
    pub fn spread(&self) -> Option<Decimal> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => Some(ask - bid),
            _ => None,
        }
    }

    /// Mid price = (best_bid + best_ask) / 2. `None` if either side is empty.
    #[must_use]
    pub fn mid_price(&self) -> Option<Decimal> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => Some((bid + ask) / Decimal::TWO),
            _ => None,
        }
    }

    /// Total number of orders currently in the book.
    #[must_use]
    pub fn order_count(&self) -> usize {
        self.index.len()
    }

    /// Number of distinct bid price levels.
    #[must_use]
    pub fn bid_depth(&self) -> usize {
        self.bids.len()
    }

    /// Number of distinct ask price levels.
    #[must_use]
    pub fn ask_depth(&self) -> usize {
        self.asks.len()
    }

    /// Returns `true` if the book has no orders on either side.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Check if an order exists in the book.
    #[must_use]
    pub fn contains_order(&self, order_id: &OrderId) -> bool {
        self.index.contains_key(order_id)
    }

    // =================================================================
    // Iteration (for the matcher)
    // =================================================================

    /// Iterate bid levels from best (highest) to worst.
    pub fn bid_levels(&self) -> impl Iterator<Item = &PriceLevel> {
        self.bids.values()
    }

    /// Iterate ask levels from best (lowest) to worst.
    pub fn ask_levels(&self) -> impl Iterator<Item = &PriceLevel> {
        self.asks.values()
    }

    /// Mutable access to bid levels.
    pub fn bid_levels_mut(&mut self) -> impl Iterator<Item = &mut PriceLevel> {
        self.bids.values_mut()
    }

    /// Mutable access to ask levels.
    pub fn ask_levels_mut(&mut self) -> impl Iterator<Item = &mut PriceLevel> {
        self.asks.values_mut()
    }

    // =================================================================
    // Maintenance
    // =================================================================

    /// Drain all orders from the book (used during settlement reset).
    pub fn drain_all(&mut self) -> Vec<Order> {
        self.index.clear();
        let mut all = Vec::new();
        for level in self.bids.values_mut() {
            all.extend(level.orders.drain(..));
        }
        for level in self.asks.values_mut() {
            all.extend(level.orders.drain(..));
        }
        self.bids.clear();
        self.asks.clear();
        all
    }
}

#[cfg(test)]
mod tests {
    use openmatch_types::*;
    use rust_decimal::Decimal;

    use super::*;

    fn make_order(side: OrderSide, price: Decimal, qty: Decimal) -> Order {
        Order::dummy_limit(side, price, qty)
    }

    #[test]
    fn insert_and_query_best_bid_ask() {
        let mut book = OrderBook::new(MarketPair::new("BTC", "USDT"));

        book.insert_order(make_order(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE))
            .unwrap();
        book.insert_order(make_order(OrderSide::Buy, Decimal::new(99, 0), Decimal::ONE))
            .unwrap();
        book.insert_order(make_order(OrderSide::Sell, Decimal::new(101, 0), Decimal::ONE))
            .unwrap();
        book.insert_order(make_order(OrderSide::Sell, Decimal::new(102, 0), Decimal::ONE))
            .unwrap();

        assert_eq!(book.best_bid(), Some(Decimal::new(100, 0)));
        assert_eq!(book.best_ask(), Some(Decimal::new(101, 0)));
        assert_eq!(book.spread(), Some(Decimal::ONE));
        assert_eq!(book.order_count(), 4);
    }

    #[test]
    fn cancel_order_removes_from_book() {
        let mut book = OrderBook::new(MarketPair::new("BTC", "USDT"));
        let order = make_order(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE);
        let id = order.id;

        book.insert_order(order).unwrap();
        assert_eq!(book.order_count(), 1);

        let cancelled = book.cancel_order(&id).unwrap();
        assert_eq!(cancelled.id, id);
        assert_eq!(book.order_count(), 0);
        assert!(book.is_empty());
    }

    #[test]
    fn cancel_nonexistent_order() {
        let mut book = OrderBook::new(MarketPair::new("BTC", "USDT"));
        let result = book.cancel_order(&OrderId::new());
        assert!(result.is_err());
    }

    #[test]
    fn cancel_removes_empty_level() {
        let mut book = OrderBook::new(MarketPair::new("BTC", "USDT"));
        let order = make_order(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE);
        let id = order.id;

        book.insert_order(order).unwrap();
        assert_eq!(book.bid_depth(), 1);

        book.cancel_order(&id).unwrap();
        assert_eq!(book.bid_depth(), 0);
    }

    #[test]
    fn duplicate_order_rejected() {
        let mut book = OrderBook::new(MarketPair::new("BTC", "USDT"));
        let order = make_order(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE);
        let dup = order.clone();

        book.insert_order(order).unwrap();
        let result = book.insert_order(dup);
        assert!(matches!(result, Err(OpenmatchError::DuplicateOrder(_))));
    }

    #[test]
    fn insert_batch() {
        let mut book = OrderBook::new(MarketPair::new("BTC", "USDT"));
        let orders = vec![
            make_order(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE),
            make_order(OrderSide::Sell, Decimal::new(101, 0), Decimal::ONE),
            make_order(OrderSide::Buy, Decimal::new(99, 0), Decimal::ONE),
        ];
        book.insert_batch(orders).unwrap();
        assert_eq!(book.order_count(), 3);
        assert_eq!(book.bid_depth(), 2);
        assert_eq!(book.ask_depth(), 1);
    }

    #[test]
    fn drain_all_empties_book() {
        let mut book = OrderBook::new(MarketPair::new("BTC", "USDT"));
        book.insert_order(make_order(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE))
            .unwrap();
        book.insert_order(make_order(OrderSide::Sell, Decimal::new(101, 0), Decimal::ONE))
            .unwrap();

        let drained = book.drain_all();
        assert_eq!(drained.len(), 2);
        assert!(book.is_empty());
        assert_eq!(book.bid_depth(), 0);
        assert_eq!(book.ask_depth(), 0);
    }

    #[test]
    fn bid_levels_iterate_highest_first() {
        let mut book = OrderBook::new(MarketPair::new("BTC", "USDT"));
        book.insert_order(make_order(OrderSide::Buy, Decimal::new(90, 0), Decimal::ONE))
            .unwrap();
        book.insert_order(make_order(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE))
            .unwrap();
        book.insert_order(make_order(OrderSide::Buy, Decimal::new(95, 0), Decimal::ONE))
            .unwrap();

        let prices: Vec<Decimal> = book.bid_levels().map(|l| l.price).collect();
        assert_eq!(
            prices,
            vec![Decimal::new(100, 0), Decimal::new(95, 0), Decimal::new(90, 0)]
        );
    }

    #[test]
    fn ask_levels_iterate_lowest_first() {
        let mut book = OrderBook::new(MarketPair::new("BTC", "USDT"));
        book.insert_order(make_order(OrderSide::Sell, Decimal::new(110, 0), Decimal::ONE))
            .unwrap();
        book.insert_order(make_order(OrderSide::Sell, Decimal::new(101, 0), Decimal::ONE))
            .unwrap();
        book.insert_order(make_order(OrderSide::Sell, Decimal::new(105, 0), Decimal::ONE))
            .unwrap();

        let prices: Vec<Decimal> = book.ask_levels().map(|l| l.price).collect();
        assert_eq!(
            prices,
            vec![Decimal::new(101, 0), Decimal::new(105, 0), Decimal::new(110, 0)]
        );
    }

    #[test]
    fn mid_price_calculation() {
        let mut book = OrderBook::new(MarketPair::new("BTC", "USDT"));
        book.insert_order(make_order(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE))
            .unwrap();
        book.insert_order(make_order(OrderSide::Sell, Decimal::new(102, 0), Decimal::ONE))
            .unwrap();
        assert_eq!(book.mid_price(), Some(Decimal::new(101, 0)));
    }

    #[test]
    fn empty_book() {
        let book = OrderBook::new(MarketPair::new("BTC", "USDT"));
        assert!(book.is_empty());
        assert_eq!(book.best_bid(), None);
        assert_eq!(book.best_ask(), None);
        assert_eq!(book.spread(), None);
        assert_eq!(book.mid_price(), None);
    }
}
