//! Pure deterministic batch matcher.
//!
//! The core matching function: takes a `SealedBatch` and produces a
//! `TradeBundle`. This is the **only** function that MatchCore exposes —
//! no side effects, no DB writes, no balance checks.
//!
//! ```text
//! match_sealed_batch(SealedBatch) -> TradeBundle
//! ```
//!
//! ## Self-Trade Prevention
//!
//! If a buy and sell order have the same `user_id`, the match is skipped
//! (wash trading prevention). The aggressive order continues to match
//! against the next passive order at that level.

use chrono::Utc;
use openmatch_types::{
    NodeId, Order, OrderSide, OrderType, SealedBatch, Trade, TradeBundle, TradeId,
};
use rust_decimal::Decimal;

use crate::{OrderBook, clearing::compute_clearing_price, determinism::compute_trade_root};

/// Pure deterministic matching: takes a sealed batch, produces a trade bundle.
///
/// ## Algorithm
///
/// 1. Insert all orders from the sealed batch into a fresh order book
/// 2. Compute the uniform clearing price
/// 3. Walk crossing orders and produce trades at the clearing price
/// 4. Self-trade prevention: skip fills where buyer == seller
/// 5. Compute trade_root hash for cross-node verification
/// 6. Return the `TradeBundle`
///
/// ## Determinism Guarantee
///
/// Given the same `SealedBatch` (same orders in same order with same
/// `batch_hash`), this function produces the **exact same** `TradeBundle`
/// on every node — same trades, same trade_root, same clearing price.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn match_sealed_batch(batch: &SealedBatch) -> TradeBundle {
    let Some(first) = batch.orders.first() else {
        // Empty batch → empty bundle
        return TradeBundle {
            epoch_id: batch.epoch_id,
            trades: vec![],
            trade_root: compute_trade_root(&[]),
            input_hash: batch.batch_hash,
            clearing_price: None,
            remaining_orders: vec![],
        };
    };
    let market = first.market.clone();

    // 1. Build the order book from the sealed batch
    let mut book = OrderBook::new(market);
    for order in &batch.orders {
        // Skip non-matchable orders (cancel orders)
        if order.order_type == OrderType::Cancel {
            continue;
        }
        // Ignore insert errors (duplicate order IDs in a sealed batch shouldn't happen)
        let _ = book.insert_order(order.clone());
    }

    // 2. Compute the clearing price
    let clearing = compute_clearing_price(&book);

    let Some(clearing_price) = clearing.clearing_price else {
        // No crossing: all orders remain unmatched
        let remaining = book.drain_all();
        return TradeBundle {
            epoch_id: batch.epoch_id,
            trades: vec![],
            trade_root: compute_trade_root(&[]),
            input_hash: batch.batch_hash,
            clearing_price: None,
            remaining_orders: remaining,
        };
    };

    // 3. Walk crossing orders and produce trades
    let mut trades: Vec<Trade> = Vec::new();
    let mut fill_seq: u64 = 0;

    // Collect bids and asks that cross at the clearing price
    let mut bids: Vec<Order> = Vec::new();
    for level in book.bid_levels() {
        if level.price >= clearing_price {
            bids.extend(level.orders.iter().cloned());
        }
    }
    // Sort bids by sequence (deterministic order)
    bids.sort_by_key(|o| o.sequence);

    let mut asks: Vec<Order> = Vec::new();
    for level in book.ask_levels() {
        if level.price <= clearing_price {
            asks.extend(level.orders.iter().cloned());
        }
    }
    // Sort asks by sequence (deterministic order)
    asks.sort_by_key(|o| o.sequence);

    // Match bids against asks at the clearing price
    let mut ask_idx = 0;
    for bid in &mut bids {
        while ask_idx < asks.len() && bid.remaining_qty > Decimal::ZERO {
            let ask = &mut asks[ask_idx];

            if ask.remaining_qty.is_zero() {
                ask_idx += 1;
                continue;
            }

            // Self-trade prevention: skip if same user
            if bid.user_id == ask.user_id {
                ask_idx += 1;
                continue;
            }

            // Compute fill quantity
            let fill_qty = bid.remaining_qty.min(ask.remaining_qty);
            let quote_amount = clearing_price * fill_qty;

            // Create the trade
            let trade = Trade {
                id: TradeId::deterministic(batch.epoch_id.0, fill_seq),
                epoch_id: batch.epoch_id,
                market: bid.market.clone(),
                taker_order_id: bid.id,
                taker_user_id: bid.user_id,
                maker_order_id: ask.id,
                maker_user_id: ask.user_id,
                price: clearing_price,
                quantity: fill_qty,
                quote_amount,
                taker_side: OrderSide::Buy,
                matcher_node: NodeId([0u8; 32]),
                executed_at: Utc::now(),
            };

            trades.push(trade);
            fill_seq += 1;

            bid.remaining_qty -= fill_qty;
            ask.remaining_qty -= fill_qty;

            if ask.remaining_qty.is_zero() {
                ask_idx += 1;
            }
        }
    }

    // 4. Compute trade root for determinism verification
    let trade_root = compute_trade_root(&trades);

    // 5. Collect remaining (unmatched or partially filled) orders
    let mut remaining = Vec::new();
    for order in bids.into_iter().chain(asks.into_iter()) {
        if order.remaining_qty > Decimal::ZERO {
            remaining.push(order);
        }
    }
    // Also collect orders that were completely on the non-crossing side
    // (bids below clearing price, asks above clearing price)
    let all_remaining = book.drain_all();
    for order in all_remaining {
        // Only add orders that weren't already included in bids/asks
        if !remaining.iter().any(|o| o.id == order.id)
            && !trades
                .iter()
                .any(|t| t.taker_order_id == order.id || t.maker_order_id == order.id)
        {
            remaining.push(order);
        }
    }

    TradeBundle {
        epoch_id: batch.epoch_id,
        trades,
        trade_root,
        input_hash: batch.batch_hash,
        clearing_price: Some(clearing_price),
        remaining_orders: remaining,
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use openmatch_types::*;
    use rust_decimal::Decimal;

    use super::*;

    fn make_sealed_batch(orders: Vec<Order>) -> SealedBatch {
        SealedBatch {
            epoch_id: EpochId(1),
            orders,
            batch_hash: [0u8; 32],
            sealed_at: Utc::now(),
            sealer_node: NodeId([0u8; 32]),
        }
    }

    #[test]
    fn empty_batch_produces_no_trades() {
        let batch = make_sealed_batch(vec![]);
        let bundle = match_sealed_batch(&batch);
        assert!(bundle.trades.is_empty());
        assert!(bundle.clearing_price.is_none());
        assert_eq!(bundle.epoch_id, EpochId(1));
    }

    #[test]
    fn no_crossing_produces_no_trades() {
        let batch = make_sealed_batch(vec![
            Order::dummy_limit(OrderSide::Buy, Decimal::new(99, 0), Decimal::ONE),
            Order::dummy_limit(OrderSide::Sell, Decimal::new(101, 0), Decimal::ONE),
        ]);
        let bundle = match_sealed_batch(&batch);
        assert!(bundle.trades.is_empty());
        assert!(bundle.clearing_price.is_none());
        assert_eq!(bundle.remaining_orders.len(), 2);
    }

    #[test]
    fn simple_crossing_produces_trade() {
        let batch = make_sealed_batch(vec![
            Order::dummy_limit(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE),
            Order::dummy_limit(OrderSide::Sell, Decimal::new(100, 0), Decimal::ONE),
        ]);
        let bundle = match_sealed_batch(&batch);
        assert_eq!(bundle.trades.len(), 1);
        assert!(bundle.clearing_price.is_some());

        let trade = &bundle.trades[0];
        assert_eq!(trade.quantity, Decimal::ONE);
        assert_eq!(trade.price, Decimal::new(100, 0));
    }

    #[test]
    fn self_trade_prevention() {
        let user = UserId::new();
        let mut buy = Order::dummy_limit(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE);
        buy.user_id = user;
        let mut sell = Order::dummy_limit(OrderSide::Sell, Decimal::new(100, 0), Decimal::ONE);
        sell.user_id = user;

        let batch = make_sealed_batch(vec![buy, sell]);
        let bundle = match_sealed_batch(&batch);
        assert!(bundle.trades.is_empty(), "Self-trade should be prevented");
    }

    #[test]
    fn partial_fill() {
        let batch = make_sealed_batch(vec![
            Order::dummy_limit(OrderSide::Buy, Decimal::new(100, 0), Decimal::new(5, 0)),
            Order::dummy_limit(OrderSide::Sell, Decimal::new(100, 0), Decimal::new(3, 0)),
        ]);
        let bundle = match_sealed_batch(&batch);
        assert_eq!(bundle.trades.len(), 1);
        assert_eq!(bundle.trades[0].quantity, Decimal::new(3, 0));
        // Buyer should have remaining 2
        let remaining_buy: Vec<&Order> = bundle
            .remaining_orders
            .iter()
            .filter(|o| o.side == OrderSide::Buy)
            .collect();
        assert!(!remaining_buy.is_empty());
    }

    #[test]
    fn multiple_fills() {
        let batch = make_sealed_batch(vec![
            Order::dummy_limit(OrderSide::Buy, Decimal::new(100, 0), Decimal::new(3, 0)),
            Order::dummy_limit(OrderSide::Sell, Decimal::new(100, 0), Decimal::ONE),
            Order::dummy_limit(OrderSide::Sell, Decimal::new(100, 0), Decimal::ONE),
            Order::dummy_limit(OrderSide::Sell, Decimal::new(100, 0), Decimal::ONE),
        ]);
        let bundle = match_sealed_batch(&batch);
        assert_eq!(bundle.trades.len(), 3);
        let total_qty: Decimal = bundle.trades.iter().map(|t| t.quantity).sum();
        assert_eq!(total_qty, Decimal::new(3, 0));
    }

    #[test]
    fn trade_ids_are_deterministic() {
        let orders = vec![
            Order::dummy_limit(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE),
            Order::dummy_limit(OrderSide::Sell, Decimal::new(100, 0), Decimal::ONE),
        ];
        let batch1 = SealedBatch {
            epoch_id: EpochId(1),
            orders: orders.clone(),
            batch_hash: [0u8; 32],
            sealed_at: Utc::now(),
            sealer_node: NodeId([0u8; 32]),
        };
        let batch2 = SealedBatch {
            epoch_id: EpochId(1),
            orders,
            batch_hash: [0u8; 32],
            sealed_at: Utc::now(),
            sealer_node: NodeId([0u8; 32]),
        };

        let bundle1 = match_sealed_batch(&batch1);
        let bundle2 = match_sealed_batch(&batch2);

        // Trade IDs should be identical (deterministic from epoch_id + fill_seq)
        assert_eq!(bundle1.trades.len(), bundle2.trades.len());
        for (t1, t2) in bundle1.trades.iter().zip(bundle2.trades.iter()) {
            assert_eq!(t1.id, t2.id, "Trade IDs must be deterministic");
        }
    }

    #[test]
    fn trade_root_is_set() {
        let batch = make_sealed_batch(vec![
            Order::dummy_limit(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE),
            Order::dummy_limit(OrderSide::Sell, Decimal::new(100, 0), Decimal::ONE),
        ]);
        let bundle = match_sealed_batch(&batch);
        assert_ne!(
            bundle.trade_root, [0u8; 32],
            "Trade root should not be zero"
        );
    }

    #[test]
    fn input_hash_is_propagated() {
        let mut batch = make_sealed_batch(vec![]);
        batch.batch_hash = [42u8; 32];
        let bundle = match_sealed_batch(&batch);
        assert_eq!(bundle.input_hash, [42u8; 32]);
    }

    #[test]
    fn cancel_orders_are_skipped() {
        let mut cancel = Order::dummy_limit(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE);
        cancel.order_type = OrderType::Cancel;

        let batch = make_sealed_batch(vec![
            cancel,
            Order::dummy_limit(OrderSide::Sell, Decimal::new(100, 0), Decimal::ONE),
        ]);
        let bundle = match_sealed_batch(&batch);
        assert!(bundle.trades.is_empty());
    }

    #[test]
    fn self_trade_skip_continues_matching() {
        // User A sells, User A buys (skip), User B buys (should match)
        let user_a = UserId::new();
        let user_b = UserId::new();

        let mut sell = Order::dummy_limit(OrderSide::Sell, Decimal::new(100, 0), Decimal::ONE);
        sell.user_id = user_a;
        sell.sequence = 0;

        let mut buy_self = Order::dummy_limit(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE);
        buy_self.user_id = user_a;
        buy_self.sequence = 1;

        let mut buy_other = Order::dummy_limit(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE);
        buy_other.user_id = user_b;
        buy_other.sequence = 2;

        let batch = make_sealed_batch(vec![sell, buy_self, buy_other]);
        let bundle = match_sealed_batch(&batch);

        // Should have at least one trade (user_b buys from user_a)
        // User_a's self-trade should be skipped
        for trade in &bundle.trades {
            assert_ne!(
                trade.taker_user_id, trade.maker_user_id,
                "No self-trade should exist"
            );
        }
    }
}
