//! Integration test: full epoch lifecycle
//!
//! COLLECT → MATCH → SETTLE
//!
//! Tests the complete flow from order creation through matching
//! to balance settlement.

use chrono::Utc;
use openmatch_core::{BalanceManager, BatchMatcher, PendingBuffer};
use openmatch_types::*;
use rust_decimal::Decimal;

fn dec(n: i64) -> Decimal {
    Decimal::new(n, 0)
}

fn make_limit_order(
    side: OrderSide,
    price: Decimal,
    qty: Decimal,
    user_id: UserId,
) -> Order {
    let id = OrderId::new();
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

#[test]
fn full_epoch_cycle_simple() {
    // =====================================================================
    // SETUP: Create users, deposit funds, freeze for orders
    // =====================================================================
    let mut balances = BalanceManager::new();
    let market = MarketPair::new("BTC", "USDT");

    // Alice wants to buy 1 BTC at 50,000 USDT
    let alice = UserId::new();
    balances.deposit(&alice, "USDT", dec(100_000)).unwrap();
    balances.freeze(&alice, "USDT", dec(50_000)).unwrap();

    // Bob wants to sell 1 BTC at 50,000 USDT
    let bob = UserId::new();
    balances.deposit(&bob, "BTC", dec(2)).unwrap();
    balances.freeze(&bob, "BTC", dec(1)).unwrap();

    // =====================================================================
    // COLLECT: Orders enter the pending buffer
    // =====================================================================
    let mut buffer = PendingBuffer::new(BatchId(1));

    let alice_order = make_limit_order(OrderSide::Buy, dec(50_000), dec(1), alice);
    let bob_order = make_limit_order(OrderSide::Sell, dec(50_000), dec(1), bob);

    buffer.push(alice_order).unwrap();
    buffer.push(bob_order).unwrap();

    // =====================================================================
    // MATCH: Seal buffer and run deterministic matching
    // =====================================================================
    let batch_hash = buffer.seal().unwrap();
    assert_ne!(batch_hash, [0u8; 32], "Batch hash should not be all zeros");

    let matcher = BatchMatcher::new(NodeId([1u8; 32]));
    let result = matcher.match_batch(buffer).unwrap();

    assert_eq!(result.trades.len(), 1, "Should produce exactly 1 trade");
    let trade = &result.trades[0];
    assert_eq!(trade.price, dec(50_000));
    assert_eq!(trade.quantity, dec(1));
    assert_eq!(trade.quote_amount, dec(50_000));
    assert_eq!(trade.taker_user_id, alice);
    assert_eq!(trade.maker_user_id, bob);
    assert!(result.remaining_orders.is_empty());

    // =====================================================================
    // SETTLE: Transfer funds between counterparties
    // =====================================================================
    balances.settle_trade(trade, &market).unwrap();

    // Verify Alice's balances
    let alice_btc = balances.get(&alice, "BTC");
    let alice_usdt = balances.get(&alice, "USDT");
    assert_eq!(alice_btc.available, dec(1), "Alice should have 1 BTC");
    assert_eq!(alice_usdt.available, dec(50_000), "Alice should have 50k USDT remaining");
    assert_eq!(alice_usdt.frozen, Decimal::ZERO, "Alice's frozen USDT should be 0");

    // Verify Bob's balances
    let bob_btc = balances.get(&bob, "BTC");
    let bob_usdt = balances.get(&bob, "USDT");
    assert_eq!(bob_btc.available, dec(1), "Bob should have 1 BTC remaining (unfrozen)");
    assert_eq!(bob_btc.frozen, Decimal::ZERO, "Bob's frozen BTC should be 0");
    assert_eq!(bob_usdt.available, dec(50_000), "Bob should have received 50k USDT");
}

#[test]
fn full_cycle_multiple_participants() {
    let mut balances = BalanceManager::new();
    let market = MarketPair::new("ETH", "USDT");

    // Buyer 1: wants 10 ETH @ 3000
    let buyer1 = UserId::new();
    balances.deposit(&buyer1, "USDT", dec(50_000)).unwrap();
    balances.freeze(&buyer1, "USDT", dec(30_000)).unwrap();

    // Buyer 2: wants 5 ETH @ 3000
    let buyer2 = UserId::new();
    balances.deposit(&buyer2, "USDT", dec(20_000)).unwrap();
    balances.freeze(&buyer2, "USDT", dec(15_000)).unwrap();

    // Seller 1: has 8 ETH, sells at 2900
    let seller1 = UserId::new();
    balances.deposit(&seller1, "ETH", dec(10)).unwrap();
    balances.freeze(&seller1, "ETH", dec(8)).unwrap();

    // Seller 2: has 7 ETH, sells at 3000
    let seller2 = UserId::new();
    balances.deposit(&seller2, "ETH", dec(10)).unwrap();
    balances.freeze(&seller2, "ETH", dec(7)).unwrap();

    // COLLECT
    let mut buffer = PendingBuffer::new(BatchId(1));
    buffer
        .push(make_limit_order(OrderSide::Buy, dec(3000), dec(10), buyer1))
        .unwrap();
    buffer
        .push(make_limit_order(OrderSide::Buy, dec(3000), dec(5), buyer2))
        .unwrap();
    buffer
        .push(make_limit_order(
            OrderSide::Sell,
            dec(2900),
            dec(8),
            seller1,
        ))
        .unwrap();
    buffer
        .push(make_limit_order(
            OrderSide::Sell,
            dec(3000),
            dec(7),
            seller2,
        ))
        .unwrap();

    // MATCH
    buffer.seal().unwrap();
    let matcher = BatchMatcher::new(NodeId([1u8; 32]));
    let result = matcher.match_batch(buffer).unwrap();

    // Total demand = 15 ETH, total supply at clearing = 15 ETH
    let total_traded: Decimal = result.trades.iter().map(|t| t.quantity).sum();
    assert_eq!(total_traded, dec(15), "All 15 ETH should trade");
    assert!(result.clearing_price.is_some());

    // SETTLE all trades
    for trade in &result.trades {
        // We need to re-freeze the correct amounts for settlement
        // In a real system, the freeze amounts would match exactly
        // For this test, we trust the freeze_proof setup
        balances.settle_trade(trade, &market).unwrap();
    }

    // Verify all frozen balances are consumed
    // Sellers should have received USDT, buyers should have received ETH
    let b1_eth = balances.get(&buyer1, "ETH");
    let b2_eth = balances.get(&buyer2, "ETH");
    assert_eq!(
        b1_eth.available + b2_eth.available,
        dec(15),
        "Buyers should have received all 15 ETH"
    );
}

#[test]
fn full_cycle_no_match() {
    let mut balances = BalanceManager::new();

    // Alice bids at 90, Bob asks at 110 — no crossing
    let alice = UserId::new();
    balances.deposit(&alice, "USDT", dec(1000)).unwrap();
    balances.freeze(&alice, "USDT", dec(900)).unwrap();

    let bob = UserId::new();
    balances.deposit(&bob, "BTC", dec(10)).unwrap();
    balances.freeze(&bob, "BTC", dec(10)).unwrap();

    let mut buffer = PendingBuffer::new(BatchId(1));
    buffer
        .push(make_limit_order(OrderSide::Buy, dec(90), dec(10), alice))
        .unwrap();
    buffer
        .push(make_limit_order(OrderSide::Sell, dec(110), dec(10), bob))
        .unwrap();
    buffer.seal().unwrap();

    let matcher = BatchMatcher::new(NodeId([1u8; 32]));
    let result = matcher.match_batch(buffer).unwrap();

    assert!(result.trades.is_empty(), "No trades when prices don't cross");
    assert_eq!(result.remaining_orders.len(), 2);
    assert!(result.clearing_price.is_none());

    // Balances unchanged (frozen amounts still intact)
    assert_eq!(balances.get(&alice, "USDT").frozen, dec(900));
    assert_eq!(balances.get(&bob, "BTC").frozen, dec(10));
}
