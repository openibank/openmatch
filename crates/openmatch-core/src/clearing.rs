//! Uniform clearing price computation for batch auctions.
//!
//! The clearing price maximizes matched volume. At this price:
//! - All buy orders with `effective_price >= clearing_price` are eligible
//! - All sell orders with `effective_price <= clearing_price` are eligible
//! - The matched volume is `min(eligible_demand, eligible_supply)`
//!
//! Ties are broken by choosing the price with smallest demand/supply imbalance,
//! then by preferring the higher price (benefits existing book liquidity).

use std::collections::BTreeSet;

use openmatch_types::Order;
use rust_decimal::Decimal;

/// Result of clearing price computation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClearingResult {
    /// The uniform clearing price.
    pub price: Decimal,
    /// The total volume that can be matched at this price.
    pub volume: Decimal,
    /// Total demand (buy quantity) at the clearing price.
    pub demand: Decimal,
    /// Total supply (sell quantity) at the clearing price.
    pub supply: Decimal,
}

/// Compute the uniform clearing price for a batch of buy and sell orders.
///
/// # Algorithm
///
/// 1. Collect all distinct prices from both sides
/// 2. For each candidate price `p`:
///    - `demand(p)` = sum of qty for buys where `effective_price >= p`
///    - `supply(p)` = sum of qty for sells where `effective_price <= p`
///    - `matchable(p)` = `min(demand(p), supply(p))`
/// 3. Choose the price that maximizes `matchable`
/// 4. Tie-break: smallest `|demand - supply|`, then highest price
///
/// # Returns
///
/// `Some(ClearingResult)` if there's a valid crossing, `None` if no match.
#[must_use]
pub fn compute_clearing_price(buys: &[Order], sells: &[Order]) -> Option<ClearingResult> {
    if buys.is_empty() || sells.is_empty() {
        return None;
    }

    // Collect all distinct price levels from both sides
    let mut price_set = BTreeSet::new();
    for order in buys.iter().chain(sells.iter()) {
        let p = order.effective_price();
        // Skip Decimal::MAX (market buy) as a candidate price level —
        // it would make all sells eligible but isn't a real price
        if p != Decimal::MAX {
            price_set.insert(p);
        }
    }

    if price_set.is_empty() {
        return None;
    }

    let mut best: Option<ClearingResult> = None;

    for &p in &price_set {
        // Demand at price p: sum of qty for all buys willing to pay >= p
        let demand: Decimal = buys
            .iter()
            .filter(|b| b.effective_price() >= p)
            .map(|b| b.remaining_qty)
            .sum();

        // Supply at price p: sum of qty for all sells willing to sell <= p
        let supply: Decimal = sells
            .iter()
            .filter(|s| s.effective_price() <= p)
            .map(|s| s.remaining_qty)
            .sum();

        let matchable = demand.min(supply);

        if matchable.is_zero() {
            continue;
        }

        let candidate = ClearingResult {
            price: p,
            volume: matchable,
            demand,
            supply,
        };

        let is_better = match &best {
            None => true,
            Some(current) => {
                if matchable > current.volume {
                    true
                } else if matchable == current.volume {
                    // Tie-break: prefer smallest imbalance
                    let new_imbalance = (demand - supply).abs();
                    let cur_imbalance = (current.demand - current.supply).abs();
                    if new_imbalance < cur_imbalance {
                        true
                    } else if new_imbalance == cur_imbalance {
                        // Second tie-break: prefer higher price
                        p > current.price
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
        };

        if is_better {
            best = Some(candidate);
        }
    }

    best
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use openmatch_types::*;
    use rust_decimal::Decimal;

    use super::*;

    fn buy(price: i64, qty: i64) -> Order {
        let id = OrderId::new();
        let user_id = UserId::new();
        Order {
            id,
            user_id,
            market: MarketPair::new("BTC", "USDT"),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            status: OrderStatus::Active,
            price: Some(Decimal::new(price, 0)),
            quantity: Decimal::new(qty, 0),
            remaining_qty: Decimal::new(qty, 0),
            freeze_proof: FreezeProof::dummy(
                id,
                user_id,
                "USDT",
                Decimal::new(price * qty, 0),
            ),
            batch_id: None,
            origin_node: NodeId([0u8; 32]),
            sequence: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn sell(price: i64, qty: i64) -> Order {
        let id = OrderId::new();
        let user_id = UserId::new();
        Order {
            id,
            user_id,
            market: MarketPair::new("BTC", "USDT"),
            side: OrderSide::Sell,
            order_type: OrderType::Limit,
            status: OrderStatus::Active,
            price: Some(Decimal::new(price, 0)),
            quantity: Decimal::new(qty, 0),
            remaining_qty: Decimal::new(qty, 0),
            freeze_proof: FreezeProof::dummy(
                id,
                user_id,
                "BTC",
                Decimal::new(qty, 0),
            ),
            batch_id: None,
            origin_node: NodeId([0u8; 32]),
            sequence: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn no_overlap_returns_none() {
        // Buys at 10, sells at 20 — no crossing
        let buys = vec![buy(10, 100)];
        let sells = vec![sell(20, 100)];
        assert!(compute_clearing_price(&buys, &sells).is_none());
    }

    #[test]
    fn exact_match_single_price() {
        // Buy 100@15, Sell 100@15
        let buys = vec![buy(15, 100)];
        let sells = vec![sell(15, 100)];
        let result = compute_clearing_price(&buys, &sells).unwrap();
        assert_eq!(result.price, Decimal::new(15, 0));
        assert_eq!(result.volume, Decimal::new(100, 0));
    }

    #[test]
    fn partial_fill() {
        // Buy 100@15, Sell 50@10 — only 50 can match
        let buys = vec![buy(15, 100)];
        let sells = vec![sell(10, 50)];
        let result = compute_clearing_price(&buys, &sells).unwrap();
        assert_eq!(result.volume, Decimal::new(50, 0));
    }

    #[test]
    fn multi_level_clearing() {
        // Buys: 50@20, 50@15
        // Sells: 30@10, 30@12, 40@18
        // At p=12: demand = 100 (both buys >= 12), supply = 60 (sells at 10,12) → match 60
        // At p=15: demand = 100, supply = 60 → match 60
        // At p=18: demand = 50 (only buy@20), supply = 100 → match 50
        // At p=10: demand = 100, supply = 30 → match 30
        // At p=20: demand = 50, supply = 100 → match 50
        // Best volume is 60 at p=12 or p=15
        let buys = vec![buy(20, 50), buy(15, 50)];
        let sells = vec![sell(10, 30), sell(12, 30), sell(18, 40)];
        let result = compute_clearing_price(&buys, &sells).unwrap();
        assert_eq!(result.volume, Decimal::new(60, 0));
        // Should prefer higher price (15) over lower (12) when volumes tie
        // At p=15: demand=100, supply=60, imbalance=40
        // At p=12: demand=100, supply=60, imbalance=40
        // Same imbalance, so prefer higher price → 15
        assert_eq!(result.price, Decimal::new(15, 0));
    }

    #[test]
    fn market_buy_order() {
        // Market buy (effective_price = MAX) should match any sell
        let id = OrderId::new();
        let user_id = UserId::new();
        let market_buy = Order {
            id,
            user_id,
            market: MarketPair::new("BTC", "USDT"),
            side: OrderSide::Buy,
            order_type: OrderType::Market,
            status: OrderStatus::Active,
            price: None,
            quantity: Decimal::new(10, 0),
            remaining_qty: Decimal::new(10, 0),
            freeze_proof: FreezeProof::dummy(id, user_id, "USDT", Decimal::new(1000000, 0)),
            batch_id: None,
            origin_node: NodeId([0u8; 32]),
            sequence: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let buys = vec![market_buy];
        let sells = vec![sell(100, 10)];
        let result = compute_clearing_price(&buys, &sells).unwrap();
        assert_eq!(result.price, Decimal::new(100, 0));
        assert_eq!(result.volume, Decimal::new(10, 0));
    }

    #[test]
    fn empty_buys_returns_none() {
        let sells = vec![sell(100, 10)];
        assert!(compute_clearing_price(&[], &sells).is_none());
    }

    #[test]
    fn empty_sells_returns_none() {
        let buys = vec![buy(100, 10)];
        assert!(compute_clearing_price(&buys, &[]).is_none());
    }

    #[test]
    fn single_buy_single_sell_crossing() {
        // Buy 5@100, Sell 3@90
        let buys = vec![buy(100, 5)];
        let sells = vec![sell(90, 3)];
        let result = compute_clearing_price(&buys, &sells).unwrap();
        // At p=90: demand=5, supply=3 → match 3
        // At p=100: demand=5, supply=3 → match 3
        // Same volume, same imbalance → prefer higher (100)
        assert_eq!(result.volume, Decimal::new(3, 0));
        assert_eq!(result.price, Decimal::new(100, 0));
    }

    #[test]
    fn tie_break_smallest_imbalance() {
        // Scenario where two prices give same volume but different imbalance
        // Buys: 100@20, 50@10
        // Sells: 60@15, 40@25
        // At p=10: demand=150, supply=0 → match 0
        // At p=15: demand=150, supply=60 → match 60
        // At p=20: demand=100, supply=60 → match 60
        // At p=25: demand=100, supply=100 → match 100
        // Best volume = 100 at p=25
        let buys = vec![buy(20, 100), buy(10, 50)];
        let sells = vec![sell(15, 60), sell(25, 40)];
        let result = compute_clearing_price(&buys, &sells).unwrap();
        // At p=20: demand=100, supply=60 → match 60
        // At p=15: demand=150, supply=60 → match 60
        // Same volume at 15 and 20, imbalance at 15 = |150-60| = 90, at 20 = |100-60| = 40
        // → prefer 20 (smaller imbalance)
        assert_eq!(result.volume, Decimal::new(60, 0));
        assert_eq!(result.price, Decimal::new(20, 0));
    }
}
