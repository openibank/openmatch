//! Trade types produced by the OpenMatch batch matcher.
//!
//! A [`Trade`] is the immutable record of a fill between a taker and maker
//! at the epoch's uniform clearing price.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{EpochId, MarketPair, NodeId, OrderId, OrderSide, TradeId, UserId};

/// A trade produced by the batch matcher.
///
/// Each trade records a single fill between a taker (aggressive) and
/// maker (passive) order. All trades within an epoch execute at the
/// uniform clearing price.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    /// Globally unique trade identifier (deterministic from epoch_id + fill_seq).
    pub id: TradeId,
    /// The epoch that produced this trade.
    pub epoch_id: EpochId,
    /// The market (e.g., BTC/USDT).
    pub market: MarketPair,
    /// The aggressive (taker) order ID.
    pub taker_order_id: OrderId,
    /// The taker's user ID.
    pub taker_user_id: UserId,
    /// The passive (maker) order ID.
    pub maker_order_id: OrderId,
    /// The maker's user ID.
    pub maker_user_id: UserId,
    /// Execution price (uniform clearing price for this epoch).
    pub price: Decimal,
    /// Executed quantity in base asset.
    pub quantity: Decimal,
    /// Quote amount = price × quantity.
    pub quote_amount: Decimal,
    /// Which side the taker was on.
    pub taker_side: OrderSide,
    /// The node that produced this trade.
    pub matcher_node: NodeId,
    /// When this trade was executed.
    pub executed_at: DateTime<Utc>,
}

impl Trade {
    /// Returns the fee-relevant notional value (quote_amount).
    #[must_use]
    pub fn notional(&self) -> Decimal {
        self.quote_amount
    }

    /// Returns `true` if the taker was buying.
    #[must_use]
    pub fn taker_is_buyer(&self) -> bool {
        self.taker_side == OrderSide::Buy
    }
}

impl std::fmt::Display for Trade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Trade[{}] {} {} {} @ {} = {}",
            self.id, self.market, self.taker_side, self.quantity, self.price, self.quote_amount,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_trade() -> Trade {
        Trade {
            id: TradeId::deterministic(1, 0),
            epoch_id: EpochId(1),
            market: MarketPair::new("BTC", "USDT"),
            taker_order_id: OrderId::new(),
            taker_user_id: UserId::new(),
            maker_order_id: OrderId::new(),
            maker_user_id: UserId::new(),
            price: Decimal::new(50000, 0),
            quantity: Decimal::new(1, 0),
            quote_amount: Decimal::new(50000, 0),
            taker_side: OrderSide::Buy,
            matcher_node: NodeId([0u8; 32]),
            executed_at: Utc::now(),
        }
    }

    #[test]
    fn trade_notional() {
        let t = make_trade();
        assert_eq!(t.notional(), Decimal::new(50000, 0));
    }

    #[test]
    fn trade_taker_side() {
        let t = make_trade();
        assert!(t.taker_is_buyer());
    }

    #[test]
    fn trade_display() {
        let t = make_trade();
        let s = format!("{t}");
        assert!(s.contains("BTC/USDT"));
        assert!(s.contains("50000"));
    }

    #[test]
    fn trade_serde_roundtrip() {
        let trade = make_trade();
        let json = serde_json::to_string(&trade).unwrap();
        let back: Trade = serde_json::from_str(&json).unwrap();
        assert_eq!(trade.id, back.id);
        assert_eq!(trade.price, back.price);
        assert_eq!(trade.quantity, back.quantity);
    }
}
