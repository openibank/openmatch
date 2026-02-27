//! Integration test: determinism verification
//!
//! The core invariant of OpeniMatch: given the same sealed buffer,
//! any node must produce the exact same result_hash.

use chrono::Utc;
use openmatch_core::{BatchMatcher, PendingBuffer};
use openmatch_types::*;
use rust_decimal::Decimal;

fn dec(n: i64) -> Decimal {
    Decimal::new(n, 0)
}

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

/// Create a deterministic test scenario with fixed orders.
fn build_test_orders() -> Vec<Order> {
    vec![
        make_order(OrderSide::Buy, dec(105), dec(20)),
        make_order(OrderSide::Buy, dec(102), dec(15)),
        make_order(OrderSide::Buy, dec(100), dec(30)),
        make_order(OrderSide::Buy, dec(98), dec(10)),
        make_order(OrderSide::Sell, dec(97), dec(25)),
        make_order(OrderSide::Sell, dec(100), dec(20)),
        make_order(OrderSide::Sell, dec(103), dec(15)),
        make_order(OrderSide::Sell, dec(106), dec(10)),
    ]
}

#[test]
fn two_matchers_same_result() {
    let orders = build_test_orders();

    // Node A
    let matcher_a = BatchMatcher::new(NodeId([1u8; 32]));
    let mut buf_a = PendingBuffer::new(BatchId(100));
    for o in &orders {
        buf_a.push(o.clone()).unwrap();
    }
    buf_a.seal().unwrap();
    let result_a = matcher_a.match_batch(buf_a).unwrap();

    // Node B (different node_id, same orders)
    let matcher_b = BatchMatcher::new(NodeId([2u8; 32]));
    let mut buf_b = PendingBuffer::new(BatchId(100));
    for o in &orders {
        buf_b.push(o.clone()).unwrap();
    }
    buf_b.seal().unwrap();
    let result_b = matcher_b.match_batch(buf_b).unwrap();

    // Core determinism assertion
    assert_eq!(
        result_a.result_hash, result_b.result_hash,
        "Different nodes with same input MUST produce same result_hash.\n\
         Node A: {}\nNode B: {}",
        hex::encode(result_a.result_hash),
        hex::encode(result_b.result_hash),
    );

    // Also verify trade-level determinism
    assert_eq!(result_a.trades.len(), result_b.trades.len());
    for (ta, tb) in result_a.trades.iter().zip(result_b.trades.iter()) {
        assert_eq!(ta.id, tb.id, "Trade IDs must be identical");
        assert_eq!(ta.price, tb.price, "Trade prices must be identical");
        assert_eq!(ta.quantity, tb.quantity, "Trade quantities must be identical");
        assert_eq!(
            ta.taker_order_id, tb.taker_order_id,
            "Taker order must be identical"
        );
        assert_eq!(
            ta.maker_order_id, tb.maker_order_id,
            "Maker order must be identical"
        );
    }

    // Input hashes must also match
    assert_eq!(
        result_a.input_hash, result_b.input_hash,
        "Input hashes must be identical"
    );
}

#[test]
fn repeated_matching_same_result() {
    // Run the same batch 5 times — all must produce identical output
    let orders = build_test_orders();
    let matcher = BatchMatcher::new(NodeId([42u8; 32]));

    let mut hashes = Vec::new();
    for _ in 0..5 {
        let mut buf = PendingBuffer::new(BatchId(999));
        for o in &orders {
            buf.push(o.clone()).unwrap();
        }
        buf.seal().unwrap();
        let result = matcher.match_batch(buf).unwrap();
        hashes.push(result.result_hash);
    }

    for (i, hash) in hashes.iter().enumerate().skip(1) {
        assert_eq!(
            hashes[0], *hash,
            "Run 0 and run {i} produced different hashes"
        );
    }
}

#[test]
fn insertion_order_does_not_affect_match_outcome() {
    // Different insertion order assigns different sequence numbers,
    // so input_hash will differ. But the MATCHING OUTCOME (clearing price,
    // total volume, number of trades) must be identical because seal()
    // sorts deterministically by (side, price_priority, sequence).
    let orders = build_test_orders();
    let matcher = BatchMatcher::new(NodeId([1u8; 32]));

    // Forward order
    let mut buf1 = PendingBuffer::new(BatchId(50));
    for o in &orders {
        buf1.push(o.clone()).unwrap();
    }
    buf1.seal().unwrap();
    let result1 = matcher.match_batch(buf1).unwrap();

    // Reverse order
    let mut buf2 = PendingBuffer::new(BatchId(50));
    for o in orders.iter().rev() {
        buf2.push(o.clone()).unwrap();
    }
    buf2.seal().unwrap();
    let result2 = matcher.match_batch(buf2).unwrap();

    // Match outcomes must be equivalent
    assert_eq!(
        result1.trades.len(),
        result2.trades.len(),
        "Same orders must produce same number of trades regardless of insertion order"
    );
    assert_eq!(
        result1.clearing_price, result2.clearing_price,
        "Clearing price must be identical"
    );

    let total_vol1: rust_decimal::Decimal =
        result1.trades.iter().map(|t| t.quantity).sum();
    let total_vol2: rust_decimal::Decimal =
        result2.trades.iter().map(|t| t.quantity).sum();
    assert_eq!(
        total_vol1, total_vol2,
        "Total traded volume must be identical"
    );
    assert_eq!(
        result1.remaining_orders.len(),
        result2.remaining_orders.len(),
        "Remaining order count must be identical"
    );
}

#[test]
fn different_batch_id_different_hashes() {
    let orders = build_test_orders();
    let matcher = BatchMatcher::new(NodeId([1u8; 32]));

    let mut buf1 = PendingBuffer::new(BatchId(1));
    for o in &orders {
        buf1.push(o.clone()).unwrap();
    }
    buf1.seal().unwrap();
    let result1 = matcher.match_batch(buf1).unwrap();

    let mut buf2 = PendingBuffer::new(BatchId(2));
    for o in &orders {
        buf2.push(o.clone()).unwrap();
    }
    buf2.seal().unwrap();
    let result2 = matcher.match_batch(buf2).unwrap();

    // Different batch IDs → different hashes (domain separation)
    assert_ne!(
        result1.input_hash, result2.input_hash,
        "Different batch IDs must produce different input hashes"
    );
    assert_ne!(
        result1.result_hash, result2.result_hash,
        "Different batch IDs must produce different result hashes"
    );
}

#[test]
fn empty_batch_deterministic() {
    let matcher = BatchMatcher::new(NodeId([1u8; 32]));

    let mut buf1 = PendingBuffer::new(BatchId(0));
    buf1.seal().unwrap();
    let r1 = matcher.match_batch(buf1).unwrap();

    let mut buf2 = PendingBuffer::new(BatchId(0));
    buf2.seal().unwrap();
    let r2 = matcher.match_batch(buf2).unwrap();

    assert_eq!(r1.result_hash, r2.result_hash);
    assert_eq!(r1.input_hash, r2.input_hash);
    assert!(r1.trades.is_empty());
}
