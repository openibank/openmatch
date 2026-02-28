//! Order types for the OpenMatch matching engine.
//!
//! Every order entering MatchCore **must** have a valid SpendRight (sr_id).
//! The Security Envelope validates this before the order enters the batch.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{EpochId, MarketPair, NodeId, OrderId, SpendRightId, UserId};

/// Which side of the book this order is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub enum OrderSide {
    Buy,
    Sell,
}

impl std::fmt::Display for OrderSide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Buy => write!(f, "BUY"),
            Self::Sell => write!(f, "SELL"),
        }
    }
}

/// The type of order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub enum OrderType {
    Limit,
    Market,
    Cancel,
}

impl std::fmt::Display for OrderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Limit => write!(f, "LIMIT"),
            Self::Market => write!(f, "MARKET"),
            Self::Cancel => write!(f, "CANCEL"),
        }
    }
}

/// Lifecycle status of an order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub enum OrderStatus {
    PendingEscrow,
    Active,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
    Expired,
}

impl std::fmt::Display for OrderStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PendingEscrow => write!(f, "PENDING_ESCROW"),
            Self::Active => write!(f, "ACTIVE"),
            Self::PartiallyFilled => write!(f, "PARTIALLY_FILLED"),
            Self::Filled => write!(f, "FILLED"),
            Self::Cancelled => write!(f, "CANCELLED"),
            Self::Rejected => write!(f, "REJECTED"),
            Self::Expired => write!(f, "EXPIRED"),
        }
    }
}

/// Core order struct. References a [`SpendRightId`] for escrow proof.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    pub id: OrderId,
    pub user_id: UserId,
    pub market: MarketPair,
    pub side: OrderSide,
    pub order_type: OrderType,
    pub status: OrderStatus,
    pub price: Option<Decimal>,
    pub quantity: Decimal,
    pub remaining_qty: Decimal,
    /// Reference to the SpendRight that funds this order.
    pub sr_id: SpendRightId,
    pub epoch_id: Option<EpochId>,
    pub origin_node: NodeId,
    pub sequence: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Order {
    #[must_use]
    pub fn effective_price(&self) -> Decimal {
        match (self.order_type, self.side) {
            (OrderType::Limit, _) => self.price.unwrap_or(Decimal::ZERO),
            (OrderType::Market, OrderSide::Buy) => Decimal::MAX,
            (OrderType::Market, OrderSide::Sell) | (OrderType::Cancel, _) => Decimal::ZERO,
        }
    }

    #[must_use]
    pub fn is_matchable_at(&self, price: &Decimal) -> bool {
        match self.side {
            OrderSide::Buy => self.effective_price() >= *price,
            OrderSide::Sell => {
                self.effective_price() <= *price || self.order_type == OrderType::Market
            }
        }
    }

    #[must_use]
    pub fn is_filled(&self) -> bool {
        self.remaining_qty.is_zero()
    }

    #[must_use]
    pub fn filled_qty(&self) -> Decimal {
        self.quantity - self.remaining_qty
    }

    #[must_use]
    pub fn fill_ratio(&self) -> Decimal {
        if self.quantity.is_zero() {
            Decimal::ZERO
        } else {
            self.filled_qty() / self.quantity
        }
    }
}

/// Test helpers.
#[cfg(any(test, feature = "test-helpers"))]
impl Order {
    pub fn dummy_limit(side: OrderSide, price: Decimal, qty: Decimal) -> Self {
        Self {
            id: OrderId::new(),
            user_id: UserId::new(),
            market: MarketPair::new("BTC", "USDT"),
            side,
            order_type: OrderType::Limit,
            status: OrderStatus::Active,
            price: Some(price),
            quantity: qty,
            remaining_qty: qty,
            sr_id: SpendRightId::new(),
            epoch_id: None,
            origin_node: NodeId([0u8; 32]),
            sequence: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    pub fn dummy_limit_for_user(
        user_id: UserId,
        side: OrderSide,
        price: Decimal,
        qty: Decimal,
    ) -> Self {
        Self {
            id: OrderId::new(),
            user_id,
            market: MarketPair::new("BTC", "USDT"),
            side,
            order_type: OrderType::Limit,
            status: OrderStatus::Active,
            price: Some(price),
            quantity: qty,
            remaining_qty: qty,
            sr_id: SpendRightId::new(),
            epoch_id: None,
            origin_node: NodeId([0u8; 32]),
            sequence: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_price_limit() {
        let order = Order::dummy_limit(OrderSide::Buy, Decimal::new(50000, 0), Decimal::ONE);
        assert_eq!(order.effective_price(), Decimal::new(50000, 0));
    }

    #[test]
    fn order_side_display() {
        assert_eq!(format!("{}", OrderSide::Buy), "BUY");
        assert_eq!(format!("{}", OrderSide::Sell), "SELL");
    }

    #[test]
    fn order_side_ordering() {
        assert!(OrderSide::Buy < OrderSide::Sell);
    }

    #[test]
    fn fill_tracking() {
        let mut order =
            Order::dummy_limit(OrderSide::Buy, Decimal::new(100, 0), Decimal::new(10, 0));
        assert!(!order.is_filled());
        order.remaining_qty = Decimal::ZERO;
        assert!(order.is_filled());
        assert_eq!(order.fill_ratio(), Decimal::ONE);
    }
}
