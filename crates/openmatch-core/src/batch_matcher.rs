//! Deterministic batch matcher for the OpeniMatch epoch-based auction.
//!
//! The [`BatchMatcher`] takes a sealed [`PendingBuffer`] and produces a
//! deterministic set of [`Trade`]s using the uniform clearing price algorithm.
//!
//! # Determinism Contract
//!
//! Given the same sealed buffer (same orders, same sequence numbers, same batch ID),
//! **every node** will produce:
//! - The same set of trades (same IDs, prices, quantities)
//! - The same `result_hash`
//!
//! This is critical for cross-node consensus verification.

use openmatch_types::*;
use sha2::{Digest, Sha256};

use crate::clearing::compute_clearing_price;
use crate::pending_buffer::PendingBuffer;

/// Result of a single batch matching round.
#[derive(Debug)]
pub struct BatchResult {
    /// The batch that was matched.
    pub batch_id: BatchId,
    /// Trades produced by the matching algorithm.
    pub trades: Vec<Trade>,
    /// SHA-256 hash of the deterministic trade output.
    pub result_hash: [u8; 32],
    /// The sealed buffer hash (input hash, for verification).
    pub input_hash: [u8; 32],
    /// Orders that remain unmatched (partially filled or no crossing).
    pub remaining_orders: Vec<Order>,
    /// The uniform clearing price used, if any.
    pub clearing_price: Option<rust_decimal::Decimal>,
}

/// The deterministic batch matcher.
///
/// # Algorithm
///
/// 1. Take orders from sealed buffer
/// 2. Separate into buys/sells, exclude cancels
/// 3. Compute uniform clearing price
/// 4. Walk buys (highest price first) × sells (lowest price first)
/// 5. Fill at clearing price, emit trades
/// 6. Compute `result_hash` over trade output
///
/// See [`compute_clearing_price`] for the clearing algorithm details.
#[derive(Debug)]
pub struct BatchMatcher {
    /// This node's identity (included in trade metadata).
    pub node_id: NodeId,
}

impl BatchMatcher {
    /// Create a new matcher for the given node.
    #[must_use]
    pub fn new(node_id: NodeId) -> Self {
        Self { node_id }
    }

    /// Run deterministic matching on a sealed pending buffer.
    ///
    /// # Errors
    /// Returns `MatchingFailed` if the buffer is not sealed.
    pub fn match_batch(&self, buffer: PendingBuffer) -> Result<BatchResult> {
        let batch_id = buffer.batch_id();
        let (orders, input_hash) = buffer.take_orders()?;

        // Partition into buys and sells, excluding cancel-type orders
        let mut buys: Vec<Order> = orders
            .iter()
            .filter(|o| o.side == OrderSide::Buy && o.order_type != OrderType::Cancel)
            .cloned()
            .collect();

        let mut sells: Vec<Order> = orders
            .iter()
            .filter(|o| o.side == OrderSide::Sell && o.order_type != OrderType::Cancel)
            .cloned()
            .collect();

        // Enforce deterministic sort order:
        // Buys: highest effective_price first, then lowest sequence
        buys.sort_by(|a, b| {
            b.effective_price()
                .cmp(&a.effective_price())
                .then_with(|| a.sequence.cmp(&b.sequence))
        });

        // Sells: lowest effective_price first, then lowest sequence
        sells.sort_by(|a, b| {
            a.effective_price()
                .cmp(&b.effective_price())
                .then_with(|| a.sequence.cmp(&b.sequence))
        });

        // Compute clearing price
        let clearing = compute_clearing_price(&buys, &sells);

        let mut trades = Vec::new();
        let mut clearing_price_used = None;

        if let Some(clearing_result) = clearing {
            let cp = clearing_result.price;
            clearing_price_used = Some(cp);

            let mut sell_idx = 0;
            let mut fill_sequence: u64 = 0;

            for buy in &mut buys {
                if buy.remaining_qty.is_zero() {
                    continue;
                }
                // Only buys willing to pay >= clearing price are eligible
                if buy.effective_price() < cp {
                    break;
                }

                while sell_idx < sells.len() && buy.remaining_qty > rust_decimal::Decimal::ZERO {
                    let sell = &mut sells[sell_idx];

                    // Only sells willing to accept <= clearing price are eligible
                    if sell.effective_price() > cp {
                        break;
                    }

                    if sell.remaining_qty.is_zero() {
                        sell_idx += 1;
                        continue;
                    }

                    // ── SELF-TRADE PREVENTION (OM_ERR_502) ──────────────────
                    // An attacker with source code knows the matching order.
                    // They could place both buy and sell to wash-trade and
                    // manipulate volume/price signals. We skip same-user pairs.
                    // This is deterministic: every node skips the same pairs.
                    if buy.user_id == sell.user_id {
                        tracing::warn!(
                            user = %buy.user_id,
                            buy_order = %buy.id,
                            sell_order = %sell.id,
                            "Self-trade blocked: same user on both sides"
                        );
                        sell_idx += 1;
                        continue;
                    }

                    let fill_qty = buy.remaining_qty.min(sell.remaining_qty);
                    let quote_amount = cp
                        .checked_mul(fill_qty)
                        .unwrap_or(rust_decimal::Decimal::MAX);

                    // Deterministic trade ID: same batch + fill sequence → same ID
                    let trade_id = TradeId::deterministic(batch_id.0, fill_sequence);
                    fill_sequence += 1;

                    let trade = Trade {
                        id: trade_id,
                        batch_id,
                        market: buy.market.clone(),
                        taker_order_id: buy.id,
                        taker_user_id: buy.user_id,
                        maker_order_id: sell.id,
                        maker_user_id: sell.user_id,
                        price: cp,
                        quantity: fill_qty,
                        quote_amount,
                        taker_side: OrderSide::Buy,
                        matcher_node: self.node_id,
                        executed_at: chrono::Utc::now(),
                    };

                    buy.remaining_qty -= fill_qty;
                    sell.remaining_qty -= fill_qty;

                    tracing::debug!(
                        trade_id = %trade.id,
                        buyer = %trade.taker_user_id,
                        seller = %trade.maker_user_id,
                        price = %trade.price,
                        qty = %trade.quantity,
                        "Trade matched"
                    );

                    trades.push(trade);

                    if sell.remaining_qty.is_zero() {
                        sell_idx += 1;
                    }
                }
            }
        }

        // Compute deterministic result hash
        let result_hash = Self::compute_result_hash(batch_id, &trades);

        // Collect remaining (unfilled / partially filled) orders
        let remaining_orders: Vec<Order> = buys
            .into_iter()
            .chain(sells)
            .filter(|o| o.remaining_qty > rust_decimal::Decimal::ZERO)
            .collect();

        tracing::info!(
            batch = batch_id.0,
            trades = trades.len(),
            remaining = remaining_orders.len(),
            clearing_price = ?clearing_price_used,
            result_hash = hex::encode(result_hash),
            "Batch matching complete"
        );

        Ok(BatchResult {
            batch_id,
            trades,
            result_hash,
            input_hash,
            remaining_orders,
            clearing_price: clearing_price_used,
        })
    }

    /// Compute the deterministic result hash over the trade output.
    ///
    /// `SHA-256(domain_sep || batch_id || num_trades || for each trade: id || price || qty)`
    fn compute_result_hash(batch_id: BatchId, trades: &[Trade]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"openmatch:result:v1:");
        hasher.update(batch_id.0.to_le_bytes());
        hasher.update((trades.len() as u64).to_le_bytes());
        for trade in trades {
            hasher.update(trade.id.0.as_bytes());
            hasher.update(trade.price.to_string().as_bytes());
            hasher.update(trade.quantity.to_string().as_bytes());
            hasher.update(trade.taker_order_id.0.as_bytes());
            hasher.update(trade.maker_order_id.0.as_bytes());
        }
        hasher.finalize().into()
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use openmatch_types::*;
    use rust_decimal::Decimal;

    use super::*;
    use crate::PendingBuffer;

    fn dec(n: i64) -> Decimal {
        Decimal::new(n, 0)
    }

    fn make_order(side: OrderSide, price: Decimal, qty: Decimal) -> Order {
        let id = OrderId::new();
        let user_id = UserId::new();
        let (otype, oprice) = if price == Decimal::MAX || price == Decimal::ZERO {
            (OrderType::Market, None)
        } else {
            (OrderType::Limit, Some(price))
        };
        let asset = match side {
            OrderSide::Buy => "USDT",
            OrderSide::Sell => "BTC",
        };
        Order {
            id,
            user_id,
            market: MarketPair::new("BTC", "USDT"),
            side,
            order_type: otype,
            status: OrderStatus::Active,
            price: oprice,
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

    fn make_limit(side: OrderSide, price: i64, qty: i64) -> Order {
        make_order(side, dec(price), dec(qty))
    }

    /// Make an order with a specific user_id (for self-trade tests).
    fn make_order_for_user(
        user_id: UserId,
        side: OrderSide,
        price: Decimal,
        qty: Decimal,
    ) -> Order {
        let id = OrderId::new();
        let (otype, oprice) = if price == Decimal::MAX || price == Decimal::ZERO {
            (OrderType::Market, None)
        } else {
            (OrderType::Limit, Some(price))
        };
        let asset = match side {
            OrderSide::Buy => "USDT",
            OrderSide::Sell => "BTC",
        };
        Order {
            id,
            user_id,
            market: MarketPair::new("BTC", "USDT"),
            side,
            order_type: otype,
            status: OrderStatus::Active,
            price: oprice,
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

    fn make_matcher() -> BatchMatcher {
        BatchMatcher::new(NodeId([1u8; 32]))
    }

    #[test]
    fn empty_batch_produces_no_trades() {
        let matcher = make_matcher();
        let mut buf = PendingBuffer::new(BatchId(1));
        buf.seal().unwrap();

        let result = matcher.match_batch(buf).unwrap();
        assert!(result.trades.is_empty());
        assert!(result.remaining_orders.is_empty());
        assert!(result.clearing_price.is_none());
    }

    #[test]
    fn no_crossing_produces_no_trades() {
        let matcher = make_matcher();
        let mut buf = PendingBuffer::new(BatchId(1));
        buf.push(make_limit(OrderSide::Buy, 90, 10)).unwrap();
        buf.push(make_limit(OrderSide::Sell, 110, 10)).unwrap();
        buf.seal().unwrap();

        let result = matcher.match_batch(buf).unwrap();
        assert!(result.trades.is_empty());
        assert_eq!(result.remaining_orders.len(), 2);
    }

    #[test]
    fn one_to_one_exact_match() {
        let matcher = make_matcher();
        let mut buf = PendingBuffer::new(BatchId(1));
        buf.push(make_limit(OrderSide::Buy, 100, 5)).unwrap();
        buf.push(make_limit(OrderSide::Sell, 100, 5)).unwrap();
        buf.seal().unwrap();

        let result = matcher.match_batch(buf).unwrap();
        assert_eq!(result.trades.len(), 1);
        assert_eq!(result.trades[0].price, dec(100));
        assert_eq!(result.trades[0].quantity, dec(5));
        assert_eq!(result.trades[0].quote_amount, dec(500));
        assert!(result.remaining_orders.is_empty());
    }

    #[test]
    fn one_buy_multiple_sells() {
        let matcher = make_matcher();
        let mut buf = PendingBuffer::new(BatchId(1));
        buf.push(make_limit(OrderSide::Buy, 100, 10)).unwrap();
        buf.push(make_limit(OrderSide::Sell, 95, 3)).unwrap();
        buf.push(make_limit(OrderSide::Sell, 98, 4)).unwrap();
        buf.push(make_limit(OrderSide::Sell, 100, 5)).unwrap();
        buf.seal().unwrap();

        let result = matcher.match_batch(buf).unwrap();
        // All sells at or below clearing price should match
        let total_qty: Decimal = result.trades.iter().map(|t| t.quantity).sum();
        assert_eq!(total_qty, dec(10), "Large buy should consume all eligible sells");
    }

    #[test]
    fn multiple_buys_one_sell() {
        let matcher = make_matcher();
        let mut buf = PendingBuffer::new(BatchId(1));
        buf.push(make_limit(OrderSide::Buy, 105, 3)).unwrap();
        buf.push(make_limit(OrderSide::Buy, 102, 4)).unwrap();
        buf.push(make_limit(OrderSide::Buy, 100, 5)).unwrap();
        buf.push(make_limit(OrderSide::Sell, 100, 10)).unwrap();
        buf.seal().unwrap();

        let result = matcher.match_batch(buf).unwrap();
        let total_qty: Decimal = result.trades.iter().map(|t| t.quantity).sum();
        assert_eq!(total_qty, dec(10));
    }

    #[test]
    fn partial_fill_leaves_remainder() {
        let matcher = make_matcher();
        let mut buf = PendingBuffer::new(BatchId(1));
        buf.push(make_limit(OrderSide::Buy, 100, 10)).unwrap();
        buf.push(make_limit(OrderSide::Sell, 100, 3)).unwrap();
        buf.seal().unwrap();

        let result = matcher.match_batch(buf).unwrap();
        assert_eq!(result.trades.len(), 1);
        assert_eq!(result.trades[0].quantity, dec(3));
        assert_eq!(result.remaining_orders.len(), 1);
        assert_eq!(result.remaining_orders[0].remaining_qty, dec(7));
    }

    #[test]
    fn price_time_priority() {
        // Two buys at same price — earlier sequence should fill first
        let matcher = make_matcher();
        let mut buf = PendingBuffer::new(BatchId(1));

        let buy1 = make_limit(OrderSide::Buy, 100, 5);
        let buy1_id = buy1.id;
        buf.push(buy1).unwrap(); // seq 0

        let buy2 = make_limit(OrderSide::Buy, 100, 5);
        let buy2_id = buy2.id;
        buf.push(buy2).unwrap(); // seq 1

        buf.push(make_limit(OrderSide::Sell, 100, 3)).unwrap();
        buf.seal().unwrap();

        let result = matcher.match_batch(buf).unwrap();
        assert_eq!(result.trades.len(), 1);
        // First buy (seq 0) should fill first
        assert_eq!(result.trades[0].taker_order_id, buy1_id);
        // buy1 partially filled (3 of 5), buy2 untouched
        let remaining_ids: Vec<OrderId> = result.remaining_orders.iter().map(|o| o.id).collect();
        assert!(remaining_ids.contains(&buy1_id)); // 2 remaining
        assert!(remaining_ids.contains(&buy2_id)); // 5 remaining
    }

    #[test]
    fn cancel_orders_excluded() {
        let matcher = make_matcher();
        let mut buf = PendingBuffer::new(BatchId(1));
        buf.push(make_limit(OrderSide::Buy, 100, 5)).unwrap();

        let mut cancel = make_limit(OrderSide::Sell, 100, 5);
        cancel.order_type = OrderType::Cancel;
        buf.push(cancel).unwrap();

        buf.seal().unwrap();

        let result = matcher.match_batch(buf).unwrap();
        // Cancel should not match against the buy
        assert!(result.trades.is_empty());
    }

    #[test]
    fn determinism_same_input_same_output() {
        // Create identical buffers and match them independently
        let orders_template: Vec<(OrderSide, i64, i64)> = vec![
            (OrderSide::Buy, 100, 10),
            (OrderSide::Buy, 99, 5),
            (OrderSide::Sell, 98, 8),
            (OrderSide::Sell, 100, 12),
        ];

        // Build orders with fixed IDs for reproducibility
        let mut orders1 = Vec::new();
        let mut orders2 = Vec::new();
        for (side, price, qty) in &orders_template {
            let o = make_limit(*side, *price, *qty);
            orders1.push(o.clone());
            orders2.push(o);
        }

        let matcher = make_matcher();

        let mut buf1 = PendingBuffer::new(BatchId(42));
        for o in orders1 {
            buf1.push(o).unwrap();
        }
        buf1.seal().unwrap();

        let mut buf2 = PendingBuffer::new(BatchId(42));
        for o in orders2 {
            buf2.push(o).unwrap();
        }
        buf2.seal().unwrap();

        let result1 = matcher.match_batch(buf1).unwrap();
        let result2 = matcher.match_batch(buf2).unwrap();

        assert_eq!(
            result1.result_hash, result2.result_hash,
            "Same input must produce same result hash"
        );
        assert_eq!(result1.trades.len(), result2.trades.len());
        for (t1, t2) in result1.trades.iter().zip(result2.trades.iter()) {
            assert_eq!(t1.id, t2.id);
            assert_eq!(t1.price, t2.price);
            assert_eq!(t1.quantity, t2.quantity);
        }
    }

    #[test]
    fn uniform_clearing_price_applied() {
        // All trades should execute at the clearing price, not individual order prices
        let matcher = make_matcher();
        let mut buf = PendingBuffer::new(BatchId(1));
        buf.push(make_limit(OrderSide::Buy, 110, 5)).unwrap(); // willing to pay 110
        buf.push(make_limit(OrderSide::Sell, 90, 5)).unwrap(); // willing to sell at 90
        buf.seal().unwrap();

        let result = matcher.match_batch(buf).unwrap();
        assert_eq!(result.trades.len(), 1);
        // Clearing price should be between 90 and 110
        let cp = result.trades[0].price;
        assert!(cp >= dec(90) && cp <= dec(110));
        // Both trades should be at the same clearing price
        assert!(result.clearing_price.is_some());
    }

    // ================================================================
    // SELF-TRADE PREVENTION TESTS (Open-Source Security Hardening)
    // ================================================================

    #[test]
    fn self_trade_blocked_same_user_both_sides() {
        // Attack: user places both buy and sell to wash-trade
        let matcher = make_matcher();
        let attacker = UserId::new();

        let mut buf = PendingBuffer::new(BatchId(1));
        buf.push(make_order_for_user(attacker, OrderSide::Buy, dec(100), dec(5)))
            .unwrap();
        buf.push(make_order_for_user(attacker, OrderSide::Sell, dec(100), dec(5)))
            .unwrap();
        buf.seal().unwrap();

        let result = matcher.match_batch(buf).unwrap();
        assert!(
            result.trades.is_empty(),
            "Self-trade must be blocked: attacker cannot trade with themselves"
        );
        // Both orders remain unmatched
        assert_eq!(result.remaining_orders.len(), 2);
    }

    #[test]
    fn self_trade_skipped_but_legitimate_trades_proceed() {
        // Attack: attacker has self-trade pair, but legitimate users also exist
        let matcher = make_matcher();
        let attacker = UserId::new();
        let honest_seller = UserId::new();

        let mut buf = PendingBuffer::new(BatchId(1));
        // Attacker's buy
        buf.push(make_order_for_user(attacker, OrderSide::Buy, dec(100), dec(5)))
            .unwrap();
        // Attacker's sell (self-trade attempt)
        buf.push(make_order_for_user(attacker, OrderSide::Sell, dec(100), dec(5)))
            .unwrap();
        // Honest seller
        buf.push(make_order_for_user(honest_seller, OrderSide::Sell, dec(100), dec(3)))
            .unwrap();
        buf.seal().unwrap();

        let result = matcher.match_batch(buf).unwrap();
        // Only the legitimate trade should execute
        assert_eq!(result.trades.len(), 1, "Only attacker-vs-honest trade should match");
        assert_eq!(result.trades[0].quantity, dec(3));
        // Verify it's the honest seller
        assert_eq!(result.trades[0].maker_user_id, honest_seller);
        assert_eq!(result.trades[0].taker_user_id, attacker);
    }

    #[test]
    fn self_trade_deterministic_across_matchers() {
        // Verify self-trade prevention is deterministic across nodes
        let attacker = UserId::new();
        let honest = UserId::new();

        let mut orders = Vec::new();
        orders.push(make_order_for_user(attacker, OrderSide::Buy, dec(100), dec(10)));
        orders.push(make_order_for_user(attacker, OrderSide::Sell, dec(100), dec(5)));
        orders.push(make_order_for_user(honest, OrderSide::Sell, dec(100), dec(8)));

        // Match on two different "nodes"
        let matcher_a = BatchMatcher::new(NodeId([1u8; 32]));
        let matcher_b = BatchMatcher::new(NodeId([2u8; 32]));

        let mut buf1 = PendingBuffer::new(BatchId(99));
        let mut buf2 = PendingBuffer::new(BatchId(99));
        for o in &orders {
            buf1.push(o.clone()).unwrap();
            buf2.push(o.clone()).unwrap();
        }
        buf1.seal().unwrap();
        buf2.seal().unwrap();

        let r1 = matcher_a.match_batch(buf1).unwrap();
        let r2 = matcher_b.match_batch(buf2).unwrap();

        assert_eq!(r1.trades.len(), r2.trades.len());
        for (t1, t2) in r1.trades.iter().zip(r2.trades.iter()) {
            assert_eq!(t1.id, t2.id);
            assert_eq!(t1.quantity, t2.quantity);
            assert_eq!(t1.taker_user_id, t2.taker_user_id);
            assert_eq!(t1.maker_user_id, t2.maker_user_id);
        }
        // Self-trade prevention must not break result hash determinism
        assert_eq!(r1.result_hash, r2.result_hash);
    }

    #[test]
    fn result_hash_changes_with_different_input() {
        let matcher = make_matcher();

        let mut buf1 = PendingBuffer::new(BatchId(1));
        buf1.push(make_limit(OrderSide::Buy, 100, 5)).unwrap();
        buf1.push(make_limit(OrderSide::Sell, 100, 5)).unwrap();
        buf1.seal().unwrap();

        let mut buf2 = PendingBuffer::new(BatchId(1));
        buf2.push(make_limit(OrderSide::Buy, 100, 10)).unwrap();
        buf2.push(make_limit(OrderSide::Sell, 100, 10)).unwrap();
        buf2.seal().unwrap();

        let r1 = matcher.match_batch(buf1).unwrap();
        let r2 = matcher.match_batch(buf2).unwrap();

        assert_ne!(
            r1.result_hash, r2.result_hash,
            "Different inputs should produce different result hashes"
        );
    }
}
