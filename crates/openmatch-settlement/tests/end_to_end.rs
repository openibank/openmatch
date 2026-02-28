//! End-to-end integration tests across all three planes.
//!
//! These tests exercise the full epoch lifecycle:
//! Security Envelope (Ingress) -> `MatchCore` -> Finality Plane (Settlement)
//!
//! They verify that the three planes work together correctly in realistic
//! scenarios: multi-user trading, partial fills, self-trade prevention,
//! supply conservation, and idempotency.

#![allow(clippy::too_many_arguments)]

use openmatch_ingress::{BalanceManager, BatchSealer, EscrowManager, PendingBuffer, RiskKernel};
use openmatch_matchcore::match_sealed_batch;
use openmatch_settlement::Tier1Settler;
use openmatch_types::*;
use rust_decimal::Decimal;

/// Helper: full epoch pipeline — collect, seal, match, settle.
struct EpochPipeline {
    node_id: NodeId,
    epoch: EpochId,
    balance_mgr: BalanceManager,
    escrow_mgr: EscrowManager,
    risk_kernel: RiskKernel,
    pending_buf: PendingBuffer,
}

impl EpochPipeline {
    fn new(epoch: EpochId) -> Self {
        let node_id = NodeId([0u8; 32]);
        Self {
            node_id,
            epoch,
            balance_mgr: BalanceManager::new(),
            escrow_mgr: EscrowManager::new(node_id),
            risk_kernel: RiskKernel::new(),
            pending_buf: PendingBuffer::new(),
        }
    }

    fn deposit(&mut self, user: UserId, asset: &str, amount: Decimal) {
        self.balance_mgr.deposit(user, asset, amount);
    }

    fn submit_order(
        &mut self,
        user: UserId,
        side: OrderSide,
        price: Decimal,
        qty: Decimal,
        escrow_asset: &str,
        escrow_amount: Decimal,
        seq: u64,
    ) -> OrderId {
        // 1. Mint SpendRight (escrow funds)
        let order_id = OrderId::new();
        let _sr_id = self
            .escrow_mgr
            .mint(
                &mut self.balance_mgr,
                order_id,
                user,
                escrow_asset,
                escrow_amount,
                self.epoch,
            )
            .expect("Escrow mint should succeed");

        // 2. Create order
        let mut order = Order::dummy_limit(side, price, qty);
        order.id = order_id;
        order.user_id = user;
        order.sequence = seq;

        // 3. Validate through risk kernel
        self.risk_kernel
            .validate(&order)
            .expect("Risk validation should pass");

        // 4. Push into pending buffer
        self.pending_buf
            .push(order)
            .expect("Buffer push should succeed");

        order_id
    }

    fn seal_and_match(&mut self) -> TradeBundle {
        // SEAL phase
        self.pending_buf.seal().expect("Seal should succeed");
        let orders = self.pending_buf.drain().expect("Drain should succeed");
        let sealer = BatchSealer::new(self.node_id);
        let sealed_batch = sealer.seal(self.epoch, orders);

        // Verify batch hash is valid
        assert!(
            BatchSealer::verify_batch_hash(&sealed_batch),
            "Batch hash must be valid"
        );

        // MATCH phase
        match_sealed_batch(&sealed_batch)
    }
}

// =============================================================================
// Test: Simple 1:1 trade across all three planes
// =============================================================================
#[test]
fn e2e_simple_trade() {
    let mut pipeline = EpochPipeline::new(EpochId(1));

    let alice = UserId::new();
    let bob = UserId::new();

    // Fund users
    pipeline.deposit(alice, "USDT", Decimal::new(100_000, 0));
    pipeline.deposit(bob, "BTC", Decimal::new(10, 0));

    // Alice buys 1 BTC @ 50,000 USDT
    pipeline.submit_order(
        alice,
        OrderSide::Buy,
        Decimal::new(50_000, 0),
        Decimal::ONE,
        "USDT",
        Decimal::new(50_000, 0),
        0,
    );

    // Bob sells 1 BTC @ 50,000 USDT
    pipeline.submit_order(
        bob,
        OrderSide::Sell,
        Decimal::new(50_000, 0),
        Decimal::ONE,
        "BTC",
        Decimal::ONE,
        1,
    );

    // Seal & Match
    let bundle = pipeline.seal_and_match();

    assert_eq!(bundle.trades.len(), 1, "Should produce exactly 1 trade");
    assert_eq!(
        bundle.clearing_price,
        Some(Decimal::new(50_000, 0)),
        "Clearing price should be 50,000"
    );
    assert_eq!(bundle.trades[0].quantity, Decimal::ONE);

    // FINALIZE: Settle
    let mut settler = Tier1Settler::new(100);
    settler.deposit(alice, "USDT", Decimal::new(50_000, 0));
    settler
        .freeze(alice, "USDT", Decimal::new(50_000, 0))
        .unwrap();
    settler.deposit(bob, "BTC", Decimal::ONE);
    settler.freeze(bob, "BTC", Decimal::ONE).unwrap();

    for trade in &bundle.trades {
        settler.settle_trade(trade).expect("Settlement should succeed");
    }

    // Verify balances after settlement
    assert_eq!(settler.balance(alice, "BTC").available, Decimal::ONE);
    assert_eq!(settler.balance(alice, "USDT").available, Decimal::ZERO);
    assert_eq!(settler.balance(alice, "USDT").frozen, Decimal::ZERO);

    assert_eq!(
        settler.balance(bob, "USDT").available,
        Decimal::new(50_000, 0)
    );
    assert_eq!(settler.balance(bob, "BTC").available, Decimal::ZERO);
    assert_eq!(settler.balance(bob, "BTC").frozen, Decimal::ZERO);

    // Verify supply conservation
    settler.verify_supply("USDT").unwrap();
    settler.verify_supply("BTC").unwrap();
}

// =============================================================================
// Test: Multiple trades with partial fills
// =============================================================================
#[test]
fn e2e_partial_fills() {
    let mut pipeline = EpochPipeline::new(EpochId(2));

    let buyer = UserId::new();
    let seller1 = UserId::new();
    let seller2 = UserId::new();

    // Fund users
    pipeline.deposit(buyer, "USDT", Decimal::new(500_000, 0));
    pipeline.deposit(seller1, "BTC", Decimal::new(3, 0));
    pipeline.deposit(seller2, "BTC", Decimal::new(3, 0));

    // Buyer wants 5 BTC @ 50,000 USDT each
    pipeline.submit_order(
        buyer,
        OrderSide::Buy,
        Decimal::new(50_000, 0),
        Decimal::new(5, 0),
        "USDT",
        Decimal::new(250_000, 0),
        0,
    );

    // Seller1 has 3 BTC to sell
    pipeline.submit_order(
        seller1,
        OrderSide::Sell,
        Decimal::new(50_000, 0),
        Decimal::new(3, 0),
        "BTC",
        Decimal::new(3, 0),
        1,
    );

    // Seller2 has 2 BTC to sell
    pipeline.submit_order(
        seller2,
        OrderSide::Sell,
        Decimal::new(50_000, 0),
        Decimal::new(2, 0),
        "BTC",
        Decimal::new(2, 0),
        2,
    );

    let bundle = pipeline.seal_and_match();

    // Should produce 2 trades (buyer vs seller1, buyer vs seller2)
    assert_eq!(bundle.trades.len(), 2, "Should produce 2 trades");

    let total_qty: Decimal = bundle.trades.iter().map(|t| t.quantity).sum();
    assert_eq!(total_qty, Decimal::new(5, 0), "Total fill should be 5 BTC");

    // FINALIZE
    let mut settler = Tier1Settler::new(100);
    settler.deposit(buyer, "USDT", Decimal::new(250_000, 0));
    settler
        .freeze(buyer, "USDT", Decimal::new(250_000, 0))
        .unwrap();
    settler.deposit(seller1, "BTC", Decimal::new(3, 0));
    settler.freeze(seller1, "BTC", Decimal::new(3, 0)).unwrap();
    settler.deposit(seller2, "BTC", Decimal::new(2, 0));
    settler.freeze(seller2, "BTC", Decimal::new(2, 0)).unwrap();

    for trade in &bundle.trades {
        settler.settle_trade(trade).unwrap();
    }

    // Verify buyer got 5 BTC
    assert_eq!(settler.balance(buyer, "BTC").available, Decimal::new(5, 0));
    assert_eq!(settler.balance(buyer, "USDT").frozen, Decimal::ZERO);

    // Verify sellers got USDT
    let s1_usdt = settler.balance(seller1, "USDT").available;
    let s2_usdt = settler.balance(seller2, "USDT").available;
    assert_eq!(
        s1_usdt + s2_usdt,
        Decimal::new(250_000, 0),
        "Sellers should receive total 250,000 USDT"
    );

    // Supply conservation
    settler.verify_supply("USDT").unwrap();
    settler.verify_supply("BTC").unwrap();
}

// =============================================================================
// Test: Self-trade prevention across the full pipeline
// =============================================================================
#[test]
fn e2e_self_trade_prevention() {
    let mut pipeline = EpochPipeline::new(EpochId(3));

    let alice = UserId::new();

    // Fund Alice with both assets
    pipeline.deposit(alice, "USDT", Decimal::new(100_000, 0));
    pipeline.deposit(alice, "BTC", Decimal::new(5, 0));

    // Alice tries to buy and sell to herself
    pipeline.submit_order(
        alice,
        OrderSide::Buy,
        Decimal::new(50_000, 0),
        Decimal::ONE,
        "USDT",
        Decimal::new(50_000, 0),
        0,
    );

    pipeline.submit_order(
        alice,
        OrderSide::Sell,
        Decimal::new(50_000, 0),
        Decimal::ONE,
        "BTC",
        Decimal::ONE,
        1,
    );

    let bundle = pipeline.seal_and_match();

    // Self-trade should be prevented — no trades
    assert!(
        bundle.trades.is_empty(),
        "Self-trade must be blocked: got {} trades",
        bundle.trades.len()
    );
}

// =============================================================================
// Test: No crossing orders — all remain unmatched
// =============================================================================
#[test]
fn e2e_no_crossing() {
    let mut pipeline = EpochPipeline::new(EpochId(4));

    let buyer = UserId::new();
    let seller = UserId::new();

    pipeline.deposit(buyer, "USDT", Decimal::new(100_000, 0));
    pipeline.deposit(seller, "BTC", Decimal::new(5, 0));

    // Buyer bids 48,000 but seller asks 52,000 — no crossing
    pipeline.submit_order(
        buyer,
        OrderSide::Buy,
        Decimal::new(48_000, 0),
        Decimal::ONE,
        "USDT",
        Decimal::new(48_000, 0),
        0,
    );

    pipeline.submit_order(
        seller,
        OrderSide::Sell,
        Decimal::new(52_000, 0),
        Decimal::ONE,
        "BTC",
        Decimal::ONE,
        1,
    );

    let bundle = pipeline.seal_and_match();

    assert!(bundle.trades.is_empty(), "No crossing should produce no trades");
    assert!(
        bundle.clearing_price.is_none(),
        "No clearing price without crossing"
    );
    assert_eq!(
        bundle.remaining_orders.len(),
        2,
        "Both orders should remain"
    );
}

// =============================================================================
// Test: Settlement idempotency across the full pipeline
// =============================================================================
#[test]
fn e2e_settlement_idempotency() {
    let mut pipeline = EpochPipeline::new(EpochId(5));

    let alice = UserId::new();
    let bob = UserId::new();

    pipeline.deposit(alice, "USDT", Decimal::new(100_000, 0));
    pipeline.deposit(bob, "BTC", Decimal::new(10, 0));

    pipeline.submit_order(
        alice,
        OrderSide::Buy,
        Decimal::new(50_000, 0),
        Decimal::ONE,
        "USDT",
        Decimal::new(50_000, 0),
        0,
    );

    pipeline.submit_order(
        bob,
        OrderSide::Sell,
        Decimal::new(50_000, 0),
        Decimal::ONE,
        "BTC",
        Decimal::ONE,
        1,
    );

    let bundle = pipeline.seal_and_match();
    assert_eq!(bundle.trades.len(), 1);

    // Settle once
    let mut settler = Tier1Settler::new(100);
    settler.deposit(alice, "USDT", Decimal::new(50_000, 0));
    settler
        .freeze(alice, "USDT", Decimal::new(50_000, 0))
        .unwrap();
    settler.deposit(bob, "BTC", Decimal::ONE);
    settler.freeze(bob, "BTC", Decimal::ONE).unwrap();

    settler.settle_trade(&bundle.trades[0]).unwrap();

    // Attempt to settle the same trade again — must fail
    let err = settler.settle_trade(&bundle.trades[0]).unwrap_err();
    assert!(
        matches!(err, OpenmatchError::TradeAlreadySettled(_)),
        "Double settlement must be blocked"
    );
}

// =============================================================================
// Test: Deterministic matching — same sealed batch produces same result
// =============================================================================
#[test]
fn e2e_deterministic_matching() {
    let node_id = NodeId([0u8; 32]);

    // Create the same set of orders twice
    let alice = UserId::new();
    let bob = UserId::new();

    let mut orders = Vec::new();
    let mut buy = Order::dummy_limit(OrderSide::Buy, Decimal::new(50_000, 0), Decimal::ONE);
    buy.user_id = alice;
    buy.sequence = 0;
    orders.push(buy);

    let mut sell = Order::dummy_limit(OrderSide::Sell, Decimal::new(50_000, 0), Decimal::ONE);
    sell.user_id = bob;
    sell.sequence = 1;
    orders.push(sell);

    let sealer = BatchSealer::new(node_id);

    // Seal the same orders twice
    let batch1 = sealer.seal(EpochId(10), orders.clone());
    let batch2 = sealer.seal(EpochId(10), orders);

    // Both batches must have the same hash
    assert_eq!(
        batch1.batch_hash, batch2.batch_hash,
        "Same orders must produce same batch hash"
    );

    // Match both batches
    let bundle1 = match_sealed_batch(&batch1);
    let bundle2 = match_sealed_batch(&batch2);

    // Same trade count
    assert_eq!(bundle1.trades.len(), bundle2.trades.len());

    // Same trade IDs (deterministic)
    for (t1, t2) in bundle1.trades.iter().zip(bundle2.trades.iter()) {
        assert_eq!(t1.id, t2.id, "Trade IDs must be deterministic");
        assert_eq!(t1.price, t2.price);
        assert_eq!(t1.quantity, t2.quantity);
    }

    // Same trade root
    assert_eq!(
        bundle1.trade_root, bundle2.trade_root,
        "Trade roots must match"
    );
}

// =============================================================================
// Test: Risk kernel blocks invalid orders before they enter the pipeline
// =============================================================================
#[test]
fn e2e_risk_kernel_blocks_invalid() {
    let mut pipeline = EpochPipeline::new(EpochId(6));

    let user = UserId::new();
    pipeline.deposit(user, "USDT", Decimal::new(1_000_000, 0));

    // Try to submit an order exceeding the max order size (default: 100 base units)
    let order_id = OrderId::new();
    let _sr_id = pipeline
        .escrow_mgr
        .mint(
            &mut pipeline.balance_mgr,
            order_id,
            user,
            "USDT",
            Decimal::new(100_000, 0),
            pipeline.epoch,
        )
        .unwrap();

    let mut order = Order::dummy_limit(
        OrderSide::Buy,
        Decimal::new(50_000, 0),
        Decimal::new(200, 0), // 200 units — exceeds default max of 100
    );
    order.user_id = user;
    order.sequence = 0;

    let err = pipeline.risk_kernel.validate(&order).unwrap_err();
    assert!(
        matches!(err, OpenmatchError::InvalidOrder { .. }),
        "Oversized order must be rejected by risk kernel"
    );
}

// =============================================================================
// Test: Insufficient balance prevents SpendRight minting
// =============================================================================
#[test]
fn e2e_insufficient_balance_blocks_escrow() {
    let mut pipeline = EpochPipeline::new(EpochId(7));

    let user = UserId::new();
    pipeline.deposit(user, "USDT", Decimal::new(1_000, 0)); // Only 1,000 USDT

    // Try to escrow 50,000 USDT — should fail
    let result = pipeline.escrow_mgr.mint(
        &mut pipeline.balance_mgr,
        OrderId::new(),
        user,
        "USDT",
        Decimal::new(50_000, 0),
        pipeline.epoch,
    );

    assert!(
        result.is_err(),
        "Escrow must fail with insufficient balance"
    );
    assert!(matches!(
        result.unwrap_err(),
        OpenmatchError::InsufficientBalance { .. }
    ));

    // Balance should be unchanged
    let bal = pipeline.balance_mgr.balance(user, "USDT");
    assert_eq!(bal.available, Decimal::new(1_000, 0));
    assert_eq!(bal.frozen, Decimal::ZERO);
}

// =============================================================================
// Test: SpendRight lifecycle through full pipeline
// =============================================================================
#[test]
fn e2e_spend_right_lifecycle() {
    let mut pipeline = EpochPipeline::new(EpochId(8));

    let user = UserId::new();
    pipeline.deposit(user, "USDT", Decimal::new(100_000, 0));

    // Mint a SpendRight
    let sr_id = pipeline
        .escrow_mgr
        .mint(
            &mut pipeline.balance_mgr,
            OrderId::new(),
            user,
            "USDT",
            Decimal::new(50_000, 0),
            pipeline.epoch,
        )
        .unwrap();

    // SR should be ACTIVE
    assert!(pipeline.escrow_mgr.is_active(&sr_id));
    assert_eq!(pipeline.escrow_mgr.active_count(), 1);

    // Balance should show frozen funds
    let bal = pipeline.balance_mgr.balance(user, "USDT");
    assert_eq!(bal.available, Decimal::new(50_000, 0));
    assert_eq!(bal.frozen, Decimal::new(50_000, 0));

    // Release the SpendRight (cancel scenario)
    pipeline
        .escrow_mgr
        .release(&mut pipeline.balance_mgr, sr_id)
        .unwrap();

    // SR should no longer be active
    assert!(!pipeline.escrow_mgr.is_active(&sr_id));
    assert_eq!(pipeline.escrow_mgr.active_count(), 0);

    // Funds should be fully available
    let bal = pipeline.balance_mgr.balance(user, "USDT");
    assert_eq!(bal.available, Decimal::new(100_000, 0));
    assert_eq!(bal.frozen, Decimal::ZERO);

    // Cannot release again (already RELEASED)
    let err = pipeline
        .escrow_mgr
        .release(&mut pipeline.balance_mgr, sr_id)
        .unwrap_err();
    assert!(matches!(err, OpenmatchError::InvalidSpendRight { .. }));
}

// =============================================================================
// Test: Multi-user auction with different prices
// =============================================================================
#[test]
fn e2e_multi_user_auction() {
    let mut pipeline = EpochPipeline::new(EpochId(9));

    let buyer1 = UserId::new();
    let buyer2 = UserId::new();
    let seller1 = UserId::new();
    let seller2 = UserId::new();

    // Fund everyone
    pipeline.deposit(buyer1, "USDT", Decimal::new(200_000, 0));
    pipeline.deposit(buyer2, "USDT", Decimal::new(200_000, 0));
    pipeline.deposit(seller1, "BTC", Decimal::new(5, 0));
    pipeline.deposit(seller2, "BTC", Decimal::new(5, 0));

    // Buyer1 bids aggressively: 52,000
    pipeline.submit_order(
        buyer1,
        OrderSide::Buy,
        Decimal::new(52_000, 0),
        Decimal::ONE,
        "USDT",
        Decimal::new(52_000, 0),
        0,
    );

    // Buyer2 bids conservatively: 49,000
    pipeline.submit_order(
        buyer2,
        OrderSide::Buy,
        Decimal::new(49_000, 0),
        Decimal::ONE,
        "USDT",
        Decimal::new(49_000, 0),
        1,
    );

    // Seller1 asks 50,000
    pipeline.submit_order(
        seller1,
        OrderSide::Sell,
        Decimal::new(50_000, 0),
        Decimal::ONE,
        "BTC",
        Decimal::ONE,
        2,
    );

    // Seller2 asks 53,000 (above buyer1's bid)
    pipeline.submit_order(
        seller2,
        OrderSide::Sell,
        Decimal::new(53_000, 0),
        Decimal::ONE,
        "BTC",
        Decimal::ONE,
        3,
    );

    let bundle = pipeline.seal_and_match();

    // Only buyer1 (52k bid) × seller1 (50k ask) should cross
    // Buyer2 (49k) < seller1 (50k), no match
    // Seller2 (53k) > buyer1 (52k), no match
    assert!(
        !bundle.trades.is_empty(),
        "At least one trade should occur between buyer1 and seller1"
    );

    if let Some(cp) = bundle.clearing_price {
        // Clearing price should be between 50,000 and 52,000
        assert!(
            cp >= Decimal::new(50_000, 0) && cp <= Decimal::new(52_000, 0),
            "Clearing price {cp} should be between 50,000 and 52,000"
        );
    }

    // Verify no self-trades
    for trade in &bundle.trades {
        assert_ne!(
            trade.taker_user_id, trade.maker_user_id,
            "No self-trade should exist"
        );
    }
}

// =============================================================================
// Test: Batch sealer hash verification
// =============================================================================
#[test]
fn e2e_batch_integrity() {
    let node_id = NodeId([0u8; 32]);
    let sealer = BatchSealer::new(node_id);

    let mut orders = Vec::new();
    for i in 0..5 {
        let mut order = Order::dummy_limit(OrderSide::Buy, Decimal::new(100, 0), Decimal::ONE);
        order.sequence = i;
        orders.push(order);
    }

    let batch = sealer.seal(EpochId(1), orders);

    // Batch hash should verify
    assert!(BatchSealer::verify_batch_hash(&batch));

    // Tampered batch should fail
    let mut tampered = batch.clone();
    tampered.batch_hash[0] ^= 0xFF;
    assert!(!BatchSealer::verify_batch_hash(&tampered));

    // Digest should match batch
    let digest = sealer.digest(&batch);
    assert_eq!(digest.batch_hash, batch.batch_hash);
    assert_eq!(digest.epoch_id, batch.epoch_id);
    assert_eq!(digest.order_count, 5);
}

// =============================================================================
// Test: Withdraw lock prevents withdrawals during critical phases
// =============================================================================
#[test]
fn e2e_withdraw_lock_phases() {
    use openmatch_settlement::WithdrawLock;

    let mut lock = WithdrawLock::new();

    // COLLECT phase — withdrawals allowed
    lock.set_phase(EpochPhase::Collect);
    assert!(lock.check_withdraw().is_ok());

    // SEAL phase — withdrawals allowed
    lock.set_phase(EpochPhase::Seal);
    assert!(lock.check_withdraw().is_ok());

    // MATCH phase — withdrawals BLOCKED
    lock.set_phase(EpochPhase::Match);
    assert!(lock.check_withdraw().is_err());

    // FINALIZE phase — withdrawals BLOCKED
    lock.set_phase(EpochPhase::Finalize);
    assert!(lock.check_withdraw().is_err());

    // Back to COLLECT — allowed again
    lock.set_phase(EpochPhase::Collect);
    assert!(lock.check_withdraw().is_ok());
}

// =============================================================================
// Test: Empty epoch produces no errors
// =============================================================================
#[test]
fn e2e_empty_epoch() {
    let mut pipeline = EpochPipeline::new(EpochId(99));

    // No orders submitted — seal and match empty batch
    let bundle = pipeline.seal_and_match();

    assert!(bundle.trades.is_empty());
    assert!(bundle.clearing_price.is_none());
    assert!(bundle.remaining_orders.is_empty());
    assert_eq!(bundle.epoch_id, EpochId(99));
}

// =============================================================================
// Test: Buffer sealing prevents late orders
// =============================================================================
#[test]
fn e2e_sealed_buffer_rejects_late_orders() {
    let mut pipeline = EpochPipeline::new(EpochId(10));

    let user = UserId::new();
    pipeline.deposit(user, "USDT", Decimal::new(100_000, 0));

    // Submit one order
    pipeline.submit_order(
        user,
        OrderSide::Buy,
        Decimal::new(50_000, 0),
        Decimal::ONE,
        "USDT",
        Decimal::new(50_000, 0),
        0,
    );

    // Seal the buffer
    pipeline.pending_buf.seal().unwrap();

    // Try to push another order — must fail
    let late_order = Order::dummy_limit(OrderSide::Buy, Decimal::new(49_000, 0), Decimal::ONE);
    let err = pipeline.pending_buf.push(late_order).unwrap_err();
    assert!(matches!(err, OpenmatchError::BufferAlreadySealed));
}
