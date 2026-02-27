//! Determinism verification utilities for cross-node consistency.
//!
//! Every node processing the same `SealedBatch` must produce the exact
//! same `TradeBundle`. The `trade_root` is a Merkle-style hash over all
//! trades that enables quick verification without comparing full payloads.

use openmatch_types::Trade;
use sha2::{Digest, Sha256};

/// Compute the trade root hash over a set of trades.
///
/// This is a deterministic hash that depends on:
/// - Trade IDs (in order)
/// - Prices and quantities
/// - Taker/maker user IDs
///
/// The same set of trades in the same order always produces the same root.
#[must_use]
pub fn compute_trade_root(trades: &[Trade]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"openmatch:trade_root:v2:");
    hasher.update((trades.len() as u64).to_le_bytes());

    for trade in trades {
        // Hash each trade deterministically
        hasher.update(trade.id.0.as_bytes());
        hasher.update(trade.epoch_id.0.to_le_bytes());
        hasher.update(trade.taker_order_id.0.as_bytes());
        hasher.update(trade.maker_order_id.0.as_bytes());
        hasher.update(trade.taker_user_id.0.as_bytes());
        hasher.update(trade.maker_user_id.0.as_bytes());
        hasher.update(trade.price.to_string().as_bytes());
        hasher.update(trade.quantity.to_string().as_bytes());
        hasher.update(trade.quote_amount.to_string().as_bytes());
    }

    let result = hasher.finalize();
    let mut root = [0u8; 32];
    root.copy_from_slice(&result);
    root
}

/// Verify that a given trade root matches the expected hash.
///
/// Recomputes the hash from the trades and compares with the expected root.
#[must_use]
pub fn verify_trade_root(trades: &[Trade], expected_root: &[u8; 32]) -> bool {
    let actual = compute_trade_root(trades);
    actual == *expected_root
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use openmatch_types::*;
    use rust_decimal::Decimal;

    use super::*;

    fn make_trade(epoch_id: u64, fill_seq: u64) -> Trade {
        Trade {
            id: TradeId::deterministic(epoch_id, fill_seq),
            epoch_id: EpochId(epoch_id),
            market: MarketPair::new("BTC", "USDT"),
            taker_order_id: OrderId::from_bytes([1; 16]),
            taker_user_id: UserId::from_bytes([2; 16]),
            maker_order_id: OrderId::from_bytes([3; 16]),
            maker_user_id: UserId::from_bytes([4; 16]),
            price: Decimal::new(50000, 0),
            quantity: Decimal::ONE,
            quote_amount: Decimal::new(50000, 0),
            taker_side: OrderSide::Buy,
            matcher_node: NodeId([0u8; 32]),
            executed_at: Utc::now(),
        }
    }

    #[test]
    fn empty_trades_deterministic() {
        let root1 = compute_trade_root(&[]);
        let root2 = compute_trade_root(&[]);
        assert_eq!(root1, root2);
    }

    #[test]
    fn same_trades_same_root() {
        let trades = vec![make_trade(1, 0), make_trade(1, 1)];
        let root1 = compute_trade_root(&trades);
        let root2 = compute_trade_root(&trades);
        assert_eq!(root1, root2);
    }

    #[test]
    fn different_trades_different_root() {
        let trades_a = vec![make_trade(1, 0)];
        let trades_b = vec![make_trade(1, 1)];
        let root_a = compute_trade_root(&trades_a);
        let root_b = compute_trade_root(&trades_b);
        assert_ne!(root_a, root_b);
    }

    #[test]
    fn order_matters() {
        let t1 = make_trade(1, 0);
        let t2 = make_trade(1, 1);
        let root_ab = compute_trade_root(&[t1.clone(), t2.clone()]);
        let root_ba = compute_trade_root(&[t2, t1]);
        assert_ne!(root_ab, root_ba, "Order of trades must affect root hash");
    }

    #[test]
    fn verify_correct_root() {
        let trades = vec![make_trade(1, 0), make_trade(1, 1)];
        let root = compute_trade_root(&trades);
        assert!(verify_trade_root(&trades, &root));
    }

    #[test]
    fn verify_wrong_root() {
        let trades = vec![make_trade(1, 0)];
        let wrong_root = [0xAB; 32];
        assert!(!verify_trade_root(&trades, &wrong_root));
    }

    #[test]
    fn root_is_32_bytes() {
        let root = compute_trade_root(&[]);
        assert_eq!(root.len(), 32);
    }
}
