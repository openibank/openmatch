//! # Security Integration Tests
//!
//! These tests prove that OpeniMatch has **blockchain-like** double-spend
//! prevention and other critical security properties.
//!
//! ## Open-Source Threat Model
//!
//! Every test here simulates an attacker who has read the full source code
//! and is trying to exploit known code paths. The tests prove that even
//! with complete knowledge, the attacks fail.
//!
//! ## Blockchain Analogies
//!
//! | Blockchain Property    | OpeniMatch Equivalent                          |
//! |------------------------|------------------------------------------------|
//! | No double-spend        | Settlement idempotency + freeze-before-trade   |
//! | UTXO model            | Frozen balance = committed UTXO                |
//! | Block finality         | Epoch SETTLE phase = block confirmation        |
//! | Supply conservation    | `∑balances = ∑deposits - ∑withdrawals`         |
//! | Replay protection      | Nonce tracking per freeze proof                |
//! | Self-send prevention   | Self-trade blocking in batch matcher            |

use chrono::Utc;
use openmatch_core::{
    BatchMatcher, NonceTracker, OrderRateLimiter, PendingBuffer, PriceSanityChecker,
    SecuredBalanceManager, SupplyConservation,
};
use openmatch_types::*;
use rust_decimal::Decimal;
use std::collections::HashMap;

fn dec(n: i64) -> Decimal {
    Decimal::new(n, 0)
}

fn make_order_for(
    user_id: UserId,
    side: OrderSide,
    price: Decimal,
    qty: Decimal,
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

// ═══════════════════════════════════════════════════════════════════
// TEST 1: Double-Spend Prevention (Blockchain Core Property)
// ═══════════════════════════════════════════════════════════════════

#[test]
fn double_spend_prevention_like_blockchain() {
    // SCENARIO: Attacker tries to settle the same trade twice to double their funds.
    // This is the EXACT analogue of double-spending in Bitcoin.
    //
    // In Bitcoin: spending the same UTXO twice
    // In OpeniMatch: settling the same trade twice to credit funds twice

    let mut mgr = SecuredBalanceManager::new(1000);
    let alice = UserId::new(); // buyer
    let bob = UserId::new(); // seller
    let market = MarketPair::new("BTC", "USDT");

    // Setup: Alice has 100,000 USDT, Bob has 2 BTC
    mgr.deposit(&alice, "USDT", dec(100_000)).unwrap();
    mgr.freeze(&alice, "USDT", dec(50_000)).unwrap(); // frozen for order
    mgr.deposit(&bob, "BTC", dec(2)).unwrap();
    mgr.freeze(&bob, "BTC", dec(1)).unwrap(); // frozen for order

    // Trade: Alice buys 1 BTC from Bob at 50,000 USDT
    let trade = Trade {
        id: TradeId::deterministic(1, 0),
        batch_id: BatchId(1),
        market: market.clone(),
        taker_order_id: OrderId::new(),
        taker_user_id: alice,
        maker_order_id: OrderId::new(),
        maker_user_id: bob,
        price: dec(50_000),
        quantity: dec(1),
        quote_amount: dec(50_000),
        taker_side: OrderSide::Buy,
        matcher_node: NodeId([0u8; 32]),
        executed_at: Utc::now(),
    };

    // First settle: OK
    mgr.settle_trade(&trade, &market).unwrap();
    assert_eq!(mgr.get(&alice, "BTC").available, dec(1));
    assert_eq!(mgr.get(&bob, "USDT").available, dec(50_000));

    // ATTACK: Try to settle the same trade again
    let double_spend_result = mgr.settle_trade(&trade, &market);
    assert!(
        matches!(double_spend_result, Err(OpenmatchError::TradeAlreadySettled(_))),
        "Double-settlement MUST be blocked — this is like double-spend in blockchain"
    );

    // Verify balances haven't changed (no double-credit)
    assert_eq!(
        mgr.get(&alice, "BTC").available,
        dec(1),
        "Alice must NOT get 2 BTC from double-settle"
    );
    assert_eq!(
        mgr.get(&bob, "USDT").available,
        dec(50_000),
        "Bob must NOT get 100,000 USDT from double-settle"
    );
}

// ═══════════════════════════════════════════════════════════════════
// TEST 2: Freeze-Before-Trade (UTXO Model)
// ═══════════════════════════════════════════════════════════════════

#[test]
fn escrow_first_model_like_utxo() {
    // SCENARIO: An attacker tries to trade without having funds frozen.
    // In Bitcoin: you can't spend without a valid UTXO
    // In OpeniMatch: you can't trade without a valid FreezeProof

    let mut mgr = SecuredBalanceManager::new(1000);
    let attacker = UserId::new();
    let victim = UserId::new();
    let market = MarketPair::new("BTC", "USDT");

    // Victim has funds properly frozen
    mgr.deposit(&victim, "BTC", dec(1)).unwrap();
    mgr.freeze(&victim, "BTC", dec(1)).unwrap();

    // Attacker deposited USDT but did NOT freeze any
    mgr.deposit(&attacker, "USDT", dec(10_000)).unwrap();
    // attacker skips freeze step — no escrow!

    // If a trade somehow got through matching, settlement will fail
    // because attacker has no frozen USDT
    let fake_trade = Trade {
        id: TradeId::deterministic(1, 0),
        batch_id: BatchId(1),
        market: market.clone(),
        taker_order_id: OrderId::new(),
        taker_user_id: attacker,
        maker_order_id: OrderId::new(),
        maker_user_id: victim,
        price: dec(10_000),
        quantity: dec(1),
        quote_amount: dec(10_000),
        taker_side: OrderSide::Buy,
        matcher_node: NodeId([0u8; 32]),
        executed_at: Utc::now(),
    };

    let result = mgr.settle_trade(&fake_trade, &market);
    assert!(
        result.is_err(),
        "Settlement MUST fail when buyer has no frozen funds (like invalid UTXO)"
    );

    // Victim's BTC is still safe
    assert_eq!(mgr.get(&victim, "BTC").frozen, dec(1));
}

// ═══════════════════════════════════════════════════════════════════
// TEST 3: Withdraw-During-Settle Attack
// ═══════════════════════════════════════════════════════════════════

#[test]
fn withdraw_during_settle_attack_blocked() {
    // SCENARIO: Attacker deposits, freezes for a trade, then tries to
    // withdraw during SETTLE phase before the settlement debits their frozen.
    // They know the exact phase timing from reading source code.

    let mut mgr = SecuredBalanceManager::new(1000);
    let attacker = UserId::new();

    mgr.deposit(&attacker, "USDT", dec(100_000)).unwrap();
    mgr.freeze(&attacker, "USDT", dec(50_000)).unwrap();

    // During COLLECT, withdrawal is fine
    mgr.withdraw(&attacker, "USDT", dec(10_000)).unwrap();
    assert_eq!(mgr.get(&attacker, "USDT").available, dec(40_000));

    // Epoch transitions to MATCH → SETTLE
    mgr.set_phase(EpochPhase::Match);
    let result = mgr.withdraw(&attacker, "USDT", dec(40_000));
    assert!(result.is_err(), "Withdraw during MATCH must be blocked");

    mgr.set_phase(EpochPhase::Settle);
    let result = mgr.withdraw(&attacker, "USDT", dec(40_000));
    assert!(result.is_err(), "Withdraw during SETTLE must be blocked");

    // After SETTLE, withdrawal resumes
    mgr.set_phase(EpochPhase::Collect);
    let result = mgr.withdraw(&attacker, "USDT", dec(40_000));
    assert!(result.is_ok(), "Withdraw should work after SETTLE completes");
}

// ═══════════════════════════════════════════════════════════════════
// TEST 4: Self-Trade (Wash Trading) Prevention
// ═══════════════════════════════════════════════════════════════════

#[test]
fn wash_trading_blocked_even_with_source_code_knowledge() {
    // SCENARIO: Attacker knows the matching algorithm from reading source.
    // They place BOTH buy and sell at the same price to:
    // - Fake volume to attract other traders
    // - Manipulate the clearing price
    // The self-trade check prevents this.

    let attacker = UserId::new();
    let matcher = BatchMatcher::new(NodeId([1u8; 32]));

    let mut buf = PendingBuffer::new(BatchId(1));
    buf.push(make_order_for(attacker, OrderSide::Buy, dec(50_000), dec(10)))
        .unwrap();
    buf.push(make_order_for(attacker, OrderSide::Sell, dec(50_000), dec(10)))
        .unwrap();
    buf.seal().unwrap();

    let result = matcher.match_batch(buf).unwrap();
    assert!(
        result.trades.is_empty(),
        "Wash trading MUST produce zero trades — attacker cannot fake volume"
    );
}

// ═══════════════════════════════════════════════════════════════════
// TEST 5: Nonce Replay Attack (Like Transaction Replay)
// ═══════════════════════════════════════════════════════════════════

#[test]
fn nonce_replay_blocked_like_blockchain_nonce() {
    // SCENARIO: Attacker captures a valid FreezeProof from the network
    // and tries to replay it to get free escrow without actually freezing.
    // In Ethereum: each tx has a nonce that prevents replay.
    // In OpeniMatch: each FreezeProof has a nonce per node.

    let mut tracker = NonceTracker::new(100);
    let honest_node = NodeId([1u8; 32]);

    // Honest node issues proof with nonce 42
    tracker.check_and_record(&honest_node, 42).unwrap();

    // ATTACK: Replay the same nonce
    let replay_result = tracker.check_and_record(&honest_node, 42);
    assert!(
        matches!(replay_result, Err(OpenmatchError::NonceReplay { nonce: 42, .. })),
        "Nonce replay MUST be detected and blocked"
    );

    // Different nonce from same node: OK
    assert!(tracker.check_and_record(&honest_node, 43).is_ok());
}

// ═══════════════════════════════════════════════════════════════════
// TEST 6: Supply Conservation (No Coins from Thin Air)
// ═══════════════════════════════════════════════════════════════════

#[test]
fn supply_conservation_after_full_trade_cycle() {
    // SCENARIO: Verify that after deposits, trades, and settlements,
    // the total money supply is conserved. This catches bugs where
    // settlement credits both parties without properly debiting.

    let mut supply = SupplyConservation::new();
    let mut mgr = SecuredBalanceManager::new(1000);
    let alice = UserId::new();
    let bob = UserId::new();

    // Deposits
    supply.record_deposit("BTC", dec(10));
    supply.record_deposit("USDT", dec(500_000));
    mgr.deposit(&alice, "USDT", dec(500_000)).unwrap();
    mgr.deposit(&bob, "BTC", dec(10)).unwrap();

    // Freeze for trade
    mgr.freeze(&alice, "USDT", dec(100_000)).unwrap();
    mgr.freeze(&bob, "BTC", dec(2)).unwrap();

    // Trade: Alice buys 2 BTC at 50,000
    let market = MarketPair::new("BTC", "USDT");
    let trade = Trade {
        id: TradeId::deterministic(1, 0),
        batch_id: BatchId(1),
        market: market.clone(),
        taker_order_id: OrderId::new(),
        taker_user_id: alice,
        maker_order_id: OrderId::new(),
        maker_user_id: bob,
        price: dec(50_000),
        quantity: dec(2),
        quote_amount: dec(100_000),
        taker_side: OrderSide::Buy,
        matcher_node: NodeId([0u8; 32]),
        executed_at: Utc::now(),
    };
    mgr.settle_trade(&trade, &market).unwrap();

    // Verify supply conservation
    // BTC: deposited 10. Alice has 2 available, Bob has 8 available + 0 frozen = 10 ✓
    // USDT: deposited 500,000. Alice has 400,000 available, Bob has 100,000 available = 500,000 ✓
    let mut btc_total = mgr.get(&alice, "BTC").total() + mgr.get(&bob, "BTC").total();
    let mut usdt_total = mgr.get(&alice, "USDT").total() + mgr.get(&bob, "USDT").total();

    let mut actual = HashMap::new();
    actual.insert("BTC".to_string(), btc_total);
    actual.insert("USDT".to_string(), usdt_total);

    assert!(
        supply.verify(&actual).is_ok(),
        "Supply must be conserved after trade: BTC={btc_total}, USDT={usdt_total}"
    );

    // After withdrawal, still conserved
    mgr.withdraw(&alice, "USDT", dec(100_000)).unwrap();
    supply.record_withdrawal("USDT", dec(100_000));

    btc_total = mgr.get(&alice, "BTC").total() + mgr.get(&bob, "BTC").total();
    usdt_total = mgr.get(&alice, "USDT").total() + mgr.get(&bob, "USDT").total();

    let mut actual = HashMap::new();
    actual.insert("BTC".to_string(), btc_total);
    actual.insert("USDT".to_string(), usdt_total);

    assert!(supply.verify(&actual).is_ok(), "Supply must be conserved after withdrawal");
}

// ═══════════════════════════════════════════════════════════════════
// TEST 7: Cross-Agent Deposit/Withdraw Attack
// ═══════════════════════════════════════════════════════════════════

#[test]
fn cross_agent_deposit_withdraw_attack_fails() {
    // SCENARIO: Agent A deposits, Agent B tries to withdraw Agent A's funds.
    // Or: Agent A freezes funds for a trade, Agent B tries to unfreeze them.
    //
    // This is prevented by per-user balance segregation — each (UserId, Asset)
    // has its own independent balance. There's no shared pool to exploit.

    let mut mgr = SecuredBalanceManager::new(1000);
    let agent_a = UserId::new();
    let agent_b = UserId::new();

    mgr.deposit(&agent_a, "USDT", dec(100_000)).unwrap();
    mgr.freeze(&agent_a, "USDT", dec(50_000)).unwrap();

    // Agent B tries to withdraw Agent A's funds
    let result = mgr.withdraw(&agent_b, "USDT", dec(100_000));
    assert!(result.is_err(), "Agent B cannot access Agent A's funds");

    // Agent B tries to unfreeze Agent A's frozen funds
    let result = mgr.unfreeze(&agent_b, "USDT", dec(50_000));
    assert!(result.is_err(), "Agent B cannot unfreeze Agent A's escrow");

    // Agent A's balances are untouched
    assert_eq!(mgr.get(&agent_a, "USDT").available, dec(50_000));
    assert_eq!(mgr.get(&agent_a, "USDT").frozen, dec(50_000));
}

// ═══════════════════════════════════════════════════════════════════
// TEST 8: Order Flood DoS Attack
// ═══════════════════════════════════════════════════════════════════

#[test]
fn order_flood_dos_attack_mitigated() {
    // SCENARIO: Attacker reads source, sees MAX_ORDERS_PER_BATCH = 100,000.
    // They flood the buffer with garbage orders to block legitimate traders.
    // Rate limiter prevents this.

    let mut limiter = OrderRateLimiter::new(1000, 5, 20);
    let attacker = UserId::new();
    let honest_user = UserId::new();

    // Attacker floods
    for i in 0..5u64 {
        limiter.check_and_record(&attacker, 100 + i).unwrap();
    }

    // Attacker is now rate-limited
    let result = limiter.check_and_record(&attacker, 106);
    assert!(
        result.is_err(),
        "Attacker must be rate-limited after burst"
    );

    // But honest users are unaffected
    assert!(
        limiter.check_and_record(&honest_user, 100).is_ok(),
        "Honest user must not be affected by attacker's rate limit"
    );
}

// ═══════════════════════════════════════════════════════════════════
// TEST 9: Price Manipulation via Extreme Orders
// ═══════════════════════════════════════════════════════════════════

#[test]
fn extreme_price_manipulation_blocked() {
    // SCENARIO: Attacker submits order at price 1,000,000x the reference
    // to manipulate the clearing price. The price sanity checker catches this.

    let mut checker = PriceSanityChecker::new(10);
    let market = MarketPair::new("BTC", "USDT");
    checker.update_reference(&market, dec(50_000));

    // Legit order: within 10x range
    assert!(checker.check_price(&market, dec(60_000)).is_ok());

    // Attack: price at 1,000,000 (20x reference)
    let result = checker.check_price(&market, dec(1_000_000));
    assert!(
        matches!(result, Err(OpenmatchError::SuspiciousPrice { .. })),
        "Extreme high price must be rejected"
    );

    // Attack: price at 1 (50,000x below reference)
    let result = checker.check_price(&market, dec(1));
    assert!(
        matches!(result, Err(OpenmatchError::SuspiciousPrice { .. })),
        "Extreme low price must be rejected"
    );
}

// ═══════════════════════════════════════════════════════════════════
// TEST 10: Full Epoch Attack Sequence (Comprehensive)
// ═══════════════════════════════════════════════════════════════════

#[test]
fn full_epoch_attack_sequence() {
    // SCENARIO: Attacker performs a coordinated multi-step attack:
    // 1. Deposit funds
    // 2. Place order (freeze)
    // 3. Try to withdraw during MATCH (should fail)
    // 4. Trade executes, try to settle twice (should fail on 2nd)
    // 5. Try to withdraw during SETTLE (should fail)
    // 6. After SETTLE, verify all balances are correct

    let mut mgr = SecuredBalanceManager::new(1000);
    let attacker = UserId::new();
    let victim = UserId::new();
    let market = MarketPair::new("BTC", "USDT");

    // Step 1: Both deposit
    mgr.deposit(&attacker, "USDT", dec(100_000)).unwrap();
    mgr.deposit(&victim, "BTC", dec(2)).unwrap();

    // Step 2: Both freeze for trade
    mgr.freeze(&attacker, "USDT", dec(50_000)).unwrap();
    mgr.freeze(&victim, "BTC", dec(1)).unwrap();

    // Step 3: MATCH phase — attacker tries to withdraw
    mgr.set_phase(EpochPhase::Match);
    assert!(
        mgr.withdraw(&attacker, "USDT", dec(50_000)).is_err(),
        "Step 3: Withdraw during MATCH must fail"
    );

    // Step 4: Settlement
    mgr.set_phase(EpochPhase::Settle);
    let trade = Trade {
        id: TradeId::deterministic(1, 0),
        batch_id: BatchId(1),
        market: market.clone(),
        taker_order_id: OrderId::new(),
        taker_user_id: attacker,
        maker_order_id: OrderId::new(),
        maker_user_id: victim,
        price: dec(50_000),
        quantity: dec(1),
        quote_amount: dec(50_000),
        taker_side: OrderSide::Buy,
        matcher_node: NodeId([0u8; 32]),
        executed_at: Utc::now(),
    };

    mgr.settle_trade(&trade, &market).unwrap();

    // Step 4b: Double-settlement attempt
    assert!(
        mgr.settle_trade(&trade, &market).is_err(),
        "Step 4b: Double-settlement must fail"
    );

    // Step 5: Try to withdraw during SETTLE
    assert!(
        mgr.withdraw(&attacker, "USDT", dec(50_000)).is_err(),
        "Step 5: Withdraw during SETTLE must fail"
    );

    // Step 6: COLLECT phase — verify final balances
    mgr.set_phase(EpochPhase::Collect);

    // Attacker: 50,000 USDT available + 0 frozen + 1 BTC available
    assert_eq!(mgr.get(&attacker, "USDT").available, dec(50_000));
    assert_eq!(mgr.get(&attacker, "USDT").frozen, Decimal::ZERO);
    assert_eq!(mgr.get(&attacker, "BTC").available, dec(1));

    // Victim: 50,000 USDT available + 1 BTC available + 0 frozen
    assert_eq!(mgr.get(&victim, "USDT").available, dec(50_000));
    assert_eq!(mgr.get(&victim, "BTC").available, dec(1));
    assert_eq!(mgr.get(&victim, "BTC").frozen, Decimal::ZERO);

    // Now withdrawal works
    assert!(mgr.withdraw(&attacker, "USDT", dec(50_000)).is_ok());
}
