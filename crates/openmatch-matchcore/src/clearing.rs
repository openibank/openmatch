//! Clearing price computation for batch auctions.
//!
//! Given the aggregated supply and demand curves from the order book,
//! computes the uniform clearing price where supply meets demand.
//!
//! The clearing price algorithm is deterministic: same inputs → same price.

use rust_decimal::Decimal;

use crate::OrderBook;

/// Result of clearing price computation.
#[derive(Debug, Clone)]
pub struct ClearingResult {
    /// The uniform clearing price, if supply and demand cross.
    pub clearing_price: Option<Decimal>,
    /// Total matchable volume at the clearing price.
    pub matchable_volume: Decimal,
    /// Best bid that contributed to the crossing.
    pub best_bid: Option<Decimal>,
    /// Best ask that contributed to the crossing.
    pub best_ask: Option<Decimal>,
}

/// Compute the uniform clearing price for a given order book.
///
/// Algorithm:
/// 1. Walk bid levels top-down, ask levels bottom-up
/// 2. Accumulate demand (cumulative bid qty) and supply (cumulative ask qty)
/// 3. Find the price level where cumulative demand ≥ cumulative supply
/// 4. Clearing price = midpoint of the crossing bid and ask
///
/// # Returns
/// A [`ClearingResult`] with the clearing price and matchable volume.
/// If no crossing exists (best bid < best ask), `clearing_price` is `None`.
#[must_use]
pub fn compute_clearing_price(book: &OrderBook) -> ClearingResult {
    let best_bid = book.best_bid();
    let best_ask = book.best_ask();

    // No crossing possible if either side is empty or bid < ask
    match (best_bid, best_ask) {
        (Some(bid), Some(ask)) if bid >= ask => {}
        _ => {
            return ClearingResult {
                clearing_price: None,
                matchable_volume: Decimal::ZERO,
                best_bid,
                best_ask,
            };
        }
    }

    // Collect bid and ask levels for the crossing computation
    let bid_levels: Vec<(Decimal, Decimal)> = book
        .bid_levels()
        .map(|level| (level.price, level.total_quantity()))
        .collect();

    let ask_levels: Vec<(Decimal, Decimal)> = book
        .ask_levels()
        .map(|level| (level.price, level.total_quantity()))
        .collect();

    // Walk from both ends to find the crossing
    let mut cum_demand = Decimal::ZERO;
    let mut cum_supply = Decimal::ZERO;
    let mut matchable = Decimal::ZERO;

    let mut bid_idx = 0;
    let mut ask_idx = 0;

    while bid_idx < bid_levels.len() && ask_idx < ask_levels.len() {
        let (bid_price, bid_qty) = bid_levels[bid_idx];
        let (ask_price, ask_qty) = ask_levels[ask_idx];

        // No more crossing once bid < ask
        if bid_price < ask_price {
            break;
        }

        cum_demand += bid_qty;
        cum_supply += ask_qty;
        matchable = cum_demand.min(cum_supply);

        bid_idx += 1;
        ask_idx += 1;
    }

    // If we have remaining bids that cross the current ask level
    while bid_idx < bid_levels.len() && ask_idx > 0 {
        let (bid_price, bid_qty) = bid_levels[bid_idx];
        let (ask_price, _) = ask_levels[ask_idx - 1];
        if bid_price < ask_price {
            break;
        }
        cum_demand += bid_qty;
        matchable = cum_demand.min(cum_supply);
        bid_idx += 1;
    }

    if matchable.is_zero() {
        return ClearingResult {
            clearing_price: None,
            matchable_volume: Decimal::ZERO,
            best_bid,
            best_ask,
        };
    }

    // Clearing price = midpoint of best bid and best ask
    let clearing = match (best_bid, best_ask) {
        (Some(b), Some(a)) => Some((b + a) / Decimal::TWO),
        _ => None,
    };

    ClearingResult {
        clearing_price: clearing,
        matchable_volume: matchable,
        best_bid,
        best_ask,
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
    fn no_crossing_when_empty() {
        let book = OrderBook::new(MarketPair::new("BTC", "USDT"));
        let result = compute_clearing_price(&book);
        assert!(result.clearing_price.is_none());
        assert_eq!(result.matchable_volume, Decimal::ZERO);
    }

    #[test]
    fn no_crossing_when_bid_below_ask() {
        let mut book = OrderBook::new(MarketPair::new("BTC", "USDT"));
        book.insert_order(make_order(
            OrderSide::Buy,
            Decimal::new(99, 0),
            Decimal::ONE,
        ))
        .unwrap();
        book.insert_order(make_order(
            OrderSide::Sell,
            Decimal::new(101, 0),
            Decimal::ONE,
        ))
        .unwrap();
        let result = compute_clearing_price(&book);
        assert!(result.clearing_price.is_none());
    }

    #[test]
    fn crossing_at_exact_price() {
        let mut book = OrderBook::new(MarketPair::new("BTC", "USDT"));
        book.insert_order(make_order(
            OrderSide::Buy,
            Decimal::new(100, 0),
            Decimal::ONE,
        ))
        .unwrap();
        book.insert_order(make_order(
            OrderSide::Sell,
            Decimal::new(100, 0),
            Decimal::ONE,
        ))
        .unwrap();
        let result = compute_clearing_price(&book);
        assert_eq!(result.clearing_price, Some(Decimal::new(100, 0)));
        assert_eq!(result.matchable_volume, Decimal::ONE);
    }

    #[test]
    fn crossing_with_spread() {
        let mut book = OrderBook::new(MarketPair::new("BTC", "USDT"));
        // Bid at 102, ask at 98 → crossing, clearing at midpoint = 100
        book.insert_order(make_order(
            OrderSide::Buy,
            Decimal::new(102, 0),
            Decimal::ONE,
        ))
        .unwrap();
        book.insert_order(make_order(
            OrderSide::Sell,
            Decimal::new(98, 0),
            Decimal::ONE,
        ))
        .unwrap();
        let result = compute_clearing_price(&book);
        assert_eq!(result.clearing_price, Some(Decimal::new(100, 0)));
    }

    #[test]
    fn matchable_volume_limited_by_smaller_side() {
        let mut book = OrderBook::new(MarketPair::new("BTC", "USDT"));
        book.insert_order(make_order(
            OrderSide::Buy,
            Decimal::new(100, 0),
            Decimal::new(5, 0),
        ))
        .unwrap();
        book.insert_order(make_order(
            OrderSide::Sell,
            Decimal::new(100, 0),
            Decimal::new(3, 0),
        ))
        .unwrap();
        let result = compute_clearing_price(&book);
        assert_eq!(result.matchable_volume, Decimal::new(3, 0));
    }

    #[test]
    fn clearing_result_has_best_bid_ask() {
        let mut book = OrderBook::new(MarketPair::new("BTC", "USDT"));
        book.insert_order(make_order(
            OrderSide::Buy,
            Decimal::new(100, 0),
            Decimal::ONE,
        ))
        .unwrap();
        book.insert_order(make_order(
            OrderSide::Sell,
            Decimal::new(100, 0),
            Decimal::ONE,
        ))
        .unwrap();
        let result = compute_clearing_price(&book);
        assert_eq!(result.best_bid, Some(Decimal::new(100, 0)));
        assert_eq!(result.best_ask, Some(Decimal::new(100, 0)));
    }
}
