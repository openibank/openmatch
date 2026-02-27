//! In-memory balance ledger for tracking available and frozen balances.
//!
//! The [`BalanceManager`] tracks per-(user, asset) balances with two components:
//! - **Available**: can be used for new orders or withdrawn
//! - **Frozen**: locked by active orders' escrow (freeze proofs)
//!
//! The lifecycle for a trade:
//! 1. `deposit` → user deposits funds (available increases)
//! 2. `freeze` → order placed, funds move from available to frozen
//! 3. `settle_trade` → trade executed, frozen funds transferred to counterparty
//! 4. `unfreeze` → order cancelled, frozen funds return to available

use std::collections::HashMap;

use openmatch_types::*;
use rust_decimal::Decimal;

/// In-memory balance ledger for all users and assets on this node.
#[derive(Debug, Default)]
pub struct BalanceManager {
    /// `(UserId, Asset) → BalanceEntry`
    balances: HashMap<(UserId, Asset), BalanceEntry>,
}

impl BalanceManager {
    /// Create a new empty balance manager.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the balance entry for a user + asset. Returns a zero entry if absent.
    #[must_use]
    pub fn get(&self, user_id: &UserId, asset: &str) -> BalanceEntry {
        self.balances
            .get(&(*user_id, asset.to_string()))
            .cloned()
            .unwrap_or_default()
    }

    /// Get a mutable reference, creating a zero entry if absent.
    fn get_mut(&mut self, user_id: &UserId, asset: &str) -> &mut BalanceEntry {
        self.balances
            .entry((*user_id, asset.to_string()))
            .or_default()
    }

    // =================================================================
    // Core operations
    // =================================================================

    /// Deposit (credit) an amount to the available balance.
    ///
    /// # Errors
    /// Returns `InvalidOrder` if amount is not positive.
    pub fn deposit(&mut self, user_id: &UserId, asset: &str, amount: Decimal) -> Result<()> {
        if amount <= Decimal::ZERO {
            return Err(OpenmatchError::InvalidOrder {
                reason: "Deposit amount must be positive".into(),
            });
        }
        let entry = self.get_mut(user_id, asset);
        entry.available += amount;
        Ok(())
    }

    /// Withdraw from available balance.
    ///
    /// # Errors
    /// Returns `InsufficientBalance` if not enough available.
    pub fn withdraw(&mut self, user_id: &UserId, asset: &str, amount: Decimal) -> Result<()> {
        if amount <= Decimal::ZERO {
            return Err(OpenmatchError::InvalidOrder {
                reason: "Withdraw amount must be positive".into(),
            });
        }
        let entry = self.get_mut(user_id, asset);
        if entry.available < amount {
            return Err(OpenmatchError::InsufficientBalance {
                needed: amount,
                available: entry.available,
            });
        }
        entry.available -= amount;
        Ok(())
    }

    /// Freeze: move `amount` from available to frozen (for an order's escrow).
    ///
    /// # Errors
    /// Returns `InsufficientBalance` if not enough available.
    pub fn freeze(&mut self, user_id: &UserId, asset: &str, amount: Decimal) -> Result<()> {
        if amount <= Decimal::ZERO {
            return Err(OpenmatchError::InvalidOrder {
                reason: "Freeze amount must be positive".into(),
            });
        }
        let entry = self.get_mut(user_id, asset);
        if entry.available < amount {
            return Err(OpenmatchError::InsufficientBalance {
                needed: amount,
                available: entry.available,
            });
        }
        entry.available -= amount;
        entry.frozen += amount;
        Ok(())
    }

    /// Unfreeze: move `amount` from frozen back to available (order cancelled).
    ///
    /// # Errors
    /// Returns `InsufficientFrozen` if not enough frozen.
    pub fn unfreeze(&mut self, user_id: &UserId, asset: &str, amount: Decimal) -> Result<()> {
        if amount <= Decimal::ZERO {
            return Err(OpenmatchError::InvalidOrder {
                reason: "Unfreeze amount must be positive".into(),
            });
        }
        let entry = self.get_mut(user_id, asset);
        if entry.frozen < amount {
            return Err(OpenmatchError::InsufficientFrozen);
        }
        entry.frozen -= amount;
        entry.available += amount;
        Ok(())
    }

    // =================================================================
    // Settlement
    // =================================================================

    /// Settle a trade by transferring frozen funds between counterparties.
    ///
    /// For a BTC/USDT market trade:
    /// - **Buyer**: frozen USDT decreases by `quote_amount`, available BTC increases by `quantity`
    /// - **Seller**: frozen BTC decreases by `quantity`, available USDT increases by `quote_amount`
    ///
    /// The buyer/seller roles are determined by `trade.taker_side`:
    /// - If taker is Buy → taker=buyer, maker=seller
    /// - If taker is Sell → taker=seller, maker=buyer
    ///
    /// # Errors
    /// Returns `InsufficientFrozen` if either party doesn't have enough frozen balance.
    pub fn settle_trade(&mut self, trade: &Trade, market: &MarketPair) -> Result<()> {
        let base = &market.base;
        let quote = &market.quote;

        let (buyer_id, seller_id) = match trade.taker_side {
            OrderSide::Buy => (trade.taker_user_id, trade.maker_user_id),
            OrderSide::Sell => (trade.maker_user_id, trade.taker_user_id),
        };

        // Buyer: deduct frozen quote, credit available base
        {
            let buyer_quote = self.get_mut(&buyer_id, quote);
            if buyer_quote.frozen < trade.quote_amount {
                return Err(OpenmatchError::InsufficientFrozen);
            }
            buyer_quote.frozen -= trade.quote_amount;
        }
        {
            let buyer_base = self.get_mut(&buyer_id, base);
            buyer_base.available += trade.quantity;
        }

        // Seller: deduct frozen base, credit available quote
        {
            let seller_base = self.get_mut(&seller_id, base);
            if seller_base.frozen < trade.quantity {
                return Err(OpenmatchError::InsufficientFrozen);
            }
            seller_base.frozen -= trade.quantity;
        }
        {
            let seller_quote = self.get_mut(&seller_id, quote);
            seller_quote.available += trade.quote_amount;
        }

        Ok(())
    }

    // =================================================================
    // Utilities
    // =================================================================

    /// Get all balances for a specific user across all assets.
    #[must_use]
    pub fn user_balances(&self, user_id: &UserId) -> HashMap<Asset, BalanceEntry> {
        self.balances
            .iter()
            .filter(|((uid, _), _)| uid == user_id)
            .map(|((_, asset), entry)| (asset.clone(), entry.clone()))
            .collect()
    }

    /// Total number of balance entries tracked.
    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.balances.len()
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use openmatch_types::*;
    use rust_decimal::Decimal;

    use super::*;

    fn dec(n: i64) -> Decimal {
        Decimal::new(n, 0)
    }

    #[test]
    fn deposit_and_query() {
        let mut mgr = BalanceManager::new();
        let user = UserId::new();

        mgr.deposit(&user, "USDT", dec(1000)).unwrap();
        let bal = mgr.get(&user, "USDT");
        assert_eq!(bal.available, dec(1000));
        assert_eq!(bal.frozen, Decimal::ZERO);
        assert_eq!(bal.total(), dec(1000));
    }

    #[test]
    fn deposit_zero_fails() {
        let mut mgr = BalanceManager::new();
        assert!(mgr.deposit(&UserId::new(), "BTC", Decimal::ZERO).is_err());
        assert!(mgr.deposit(&UserId::new(), "BTC", dec(-1)).is_err());
    }

    #[test]
    fn withdraw_sufficient() {
        let mut mgr = BalanceManager::new();
        let user = UserId::new();
        mgr.deposit(&user, "USDT", dec(1000)).unwrap();
        mgr.withdraw(&user, "USDT", dec(300)).unwrap();
        assert_eq!(mgr.get(&user, "USDT").available, dec(700));
    }

    #[test]
    fn withdraw_insufficient() {
        let mut mgr = BalanceManager::new();
        let user = UserId::new();
        mgr.deposit(&user, "USDT", dec(100)).unwrap();
        let result = mgr.withdraw(&user, "USDT", dec(200));
        assert!(matches!(
            result,
            Err(OpenmatchError::InsufficientBalance { .. })
        ));
    }

    #[test]
    fn freeze_and_unfreeze() {
        let mut mgr = BalanceManager::new();
        let user = UserId::new();
        mgr.deposit(&user, "USDT", dec(1000)).unwrap();

        mgr.freeze(&user, "USDT", dec(400)).unwrap();
        let bal = mgr.get(&user, "USDT");
        assert_eq!(bal.available, dec(600));
        assert_eq!(bal.frozen, dec(400));
        assert_eq!(bal.total(), dec(1000));

        mgr.unfreeze(&user, "USDT", dec(400)).unwrap();
        let bal = mgr.get(&user, "USDT");
        assert_eq!(bal.available, dec(1000));
        assert_eq!(bal.frozen, Decimal::ZERO);
    }

    #[test]
    fn freeze_insufficient() {
        let mut mgr = BalanceManager::new();
        let user = UserId::new();
        mgr.deposit(&user, "USDT", dec(100)).unwrap();
        let result = mgr.freeze(&user, "USDT", dec(200));
        assert!(matches!(
            result,
            Err(OpenmatchError::InsufficientBalance { .. })
        ));
    }

    #[test]
    fn unfreeze_insufficient() {
        let mut mgr = BalanceManager::new();
        let user = UserId::new();
        mgr.deposit(&user, "USDT", dec(100)).unwrap();
        mgr.freeze(&user, "USDT", dec(50)).unwrap();
        let result = mgr.unfreeze(&user, "USDT", dec(100));
        assert!(matches!(result, Err(OpenmatchError::InsufficientFrozen)));
    }

    #[test]
    fn settle_trade_moves_funds() {
        let mut mgr = BalanceManager::new();
        let buyer = UserId::new();
        let seller = UserId::new();
        let market = MarketPair::new("BTC", "USDT");

        // Buyer has 50000 USDT frozen (for buying BTC)
        mgr.deposit(&buyer, "USDT", dec(50000)).unwrap();
        mgr.freeze(&buyer, "USDT", dec(50000)).unwrap();

        // Seller has 1 BTC frozen (for selling)
        mgr.deposit(&seller, "BTC", dec(1)).unwrap();
        mgr.freeze(&seller, "BTC", dec(1)).unwrap();

        let trade = Trade {
            id: TradeId::deterministic(1, 0),
            batch_id: BatchId(1),
            market: market.clone(),
            taker_order_id: OrderId::new(),
            taker_user_id: buyer,
            maker_order_id: OrderId::new(),
            maker_user_id: seller,
            price: dec(50000),
            quantity: dec(1),
            quote_amount: dec(50000),
            taker_side: OrderSide::Buy,
            matcher_node: NodeId([0u8; 32]),
            executed_at: Utc::now(),
        };

        mgr.settle_trade(&trade, &market).unwrap();

        // Buyer: got 1 BTC available, 0 USDT frozen remaining
        assert_eq!(mgr.get(&buyer, "BTC").available, dec(1));
        assert_eq!(mgr.get(&buyer, "USDT").frozen, Decimal::ZERO);

        // Seller: got 50000 USDT available, 0 BTC frozen remaining
        assert_eq!(mgr.get(&seller, "USDT").available, dec(50000));
        assert_eq!(mgr.get(&seller, "BTC").frozen, Decimal::ZERO);
    }

    #[test]
    fn settle_trade_taker_sell() {
        let mut mgr = BalanceManager::new();
        let taker = UserId::new(); // selling BTC
        let maker = UserId::new(); // buying BTC (had resting order)
        let market = MarketPair::new("BTC", "USDT");

        // Maker (buyer) has USDT frozen
        mgr.deposit(&maker, "USDT", dec(50000)).unwrap();
        mgr.freeze(&maker, "USDT", dec(50000)).unwrap();

        // Taker (seller) has BTC frozen
        mgr.deposit(&taker, "BTC", dec(1)).unwrap();
        mgr.freeze(&taker, "BTC", dec(1)).unwrap();

        let trade = Trade {
            id: TradeId::deterministic(1, 0),
            batch_id: BatchId(1),
            market: market.clone(),
            taker_order_id: OrderId::new(),
            taker_user_id: taker,
            maker_order_id: OrderId::new(),
            maker_user_id: maker,
            price: dec(50000),
            quantity: dec(1),
            quote_amount: dec(50000),
            taker_side: OrderSide::Sell,
            matcher_node: NodeId([0u8; 32]),
            executed_at: Utc::now(),
        };

        mgr.settle_trade(&trade, &market).unwrap();

        // Taker (seller): got USDT, spent BTC
        assert_eq!(mgr.get(&taker, "USDT").available, dec(50000));
        assert_eq!(mgr.get(&taker, "BTC").frozen, Decimal::ZERO);

        // Maker (buyer): got BTC, spent USDT
        assert_eq!(mgr.get(&maker, "BTC").available, dec(1));
        assert_eq!(mgr.get(&maker, "USDT").frozen, Decimal::ZERO);
    }

    #[test]
    fn user_balances_query() {
        let mut mgr = BalanceManager::new();
        let user = UserId::new();
        mgr.deposit(&user, "BTC", dec(5)).unwrap();
        mgr.deposit(&user, "USDT", dec(10000)).unwrap();

        let balances = mgr.user_balances(&user);
        assert_eq!(balances.len(), 2);
        assert_eq!(balances["BTC"].available, dec(5));
        assert_eq!(balances["USDT"].available, dec(10000));
    }

    #[test]
    fn nonexistent_user_returns_zero() {
        let mgr = BalanceManager::new();
        let bal = mgr.get(&UserId::new(), "BTC");
        assert!(bal.is_zero());
    }
}
