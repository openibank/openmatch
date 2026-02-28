//! Balance management for the Security Envelope.
//!
//! Tracks per-(user, asset) balances with available/frozen accounting.
//! All mutations are atomic: either the full operation succeeds or
//! the balance is unchanged.

use std::collections::HashMap;

use openmatch_types::{Asset, BalanceEntry, OpenmatchError, Result, UserId};
use rust_decimal::Decimal;

/// Manages user balances with available/frozen accounting.
///
/// The BalanceManager is the source of truth for all balance state.
/// The EscrowManager calls into it to freeze/unfreeze funds when
/// minting or releasing SpendRights.
pub struct BalanceManager {
    /// Per-(user, asset) balances.
    balances: HashMap<(UserId, Asset), BalanceEntry>,
}

impl BalanceManager {
    /// Create a new empty balance manager.
    #[must_use]
    pub fn new() -> Self {
        Self {
            balances: HashMap::new(),
        }
    }

    /// Deposit funds (increases available balance).
    pub fn deposit(&mut self, user_id: UserId, asset: &str, amount: Decimal) {
        let entry = self
            .balances
            .entry((user_id, asset.to_string()))
            .or_default();
        entry.available += amount;
    }

    /// Freeze funds (available → frozen). Used when minting a SpendRight.
    ///
    /// # Errors
    /// Returns `InsufficientBalance` if available < amount.
    pub fn freeze(&mut self, user_id: UserId, asset: &str, amount: Decimal) -> Result<()> {
        let entry = self.balances.get_mut(&(user_id, asset.to_string())).ok_or(
            OpenmatchError::InsufficientBalance {
                needed: amount,
                available: Decimal::ZERO,
            },
        )?;

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

    /// Unfreeze funds (frozen → available). Used when releasing a SpendRight.
    ///
    /// # Errors
    /// Returns `InsufficientFrozen` if frozen < amount.
    pub fn unfreeze(&mut self, user_id: UserId, asset: &str, amount: Decimal) -> Result<()> {
        let entry = self
            .balances
            .get_mut(&(user_id, asset.to_string()))
            .ok_or(OpenmatchError::InsufficientFrozen)?;

        if entry.frozen < amount {
            return Err(OpenmatchError::InsufficientFrozen);
        }

        entry.frozen -= amount;
        entry.available += amount;
        Ok(())
    }

    /// Consume frozen funds (for settlement). Frozen balance decreases,
    /// nothing is added back to available.
    ///
    /// # Errors
    /// Returns `InsufficientFrozen` if frozen < amount.
    pub fn consume_frozen(&mut self, user_id: UserId, asset: &str, amount: Decimal) -> Result<()> {
        let entry = self
            .balances
            .get_mut(&(user_id, asset.to_string()))
            .ok_or(OpenmatchError::InsufficientFrozen)?;

        if entry.frozen < amount {
            return Err(OpenmatchError::InsufficientFrozen);
        }

        entry.frozen -= amount;
        Ok(())
    }

    /// Credit available balance (for settlement — receiving side).
    pub fn credit(&mut self, user_id: UserId, asset: &str, amount: Decimal) {
        let entry = self
            .balances
            .entry((user_id, asset.to_string()))
            .or_default();
        entry.available += amount;
    }

    /// Get the balance for a (user, asset) pair.
    #[must_use]
    pub fn balance(&self, user_id: UserId, asset: &str) -> BalanceEntry {
        self.balances
            .get(&(user_id, asset.to_string()))
            .cloned()
            .unwrap_or_default()
    }

    /// Total supply of an asset (sum of all users' available + frozen).
    #[must_use]
    pub fn total_supply(&self, asset: &str) -> Decimal {
        self.balances
            .iter()
            .filter(|((_, a), _)| a == asset)
            .map(|(_, entry)| entry.total())
            .sum()
    }
}

impl Default for BalanceManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deposit_increases_available() {
        let mut bm = BalanceManager::new();
        let user = UserId::new();
        bm.deposit(user, "USDT", Decimal::new(1000, 0));
        let bal = bm.balance(user, "USDT");
        assert_eq!(bal.available, Decimal::new(1000, 0));
        assert_eq!(bal.frozen, Decimal::ZERO);
    }

    #[test]
    fn freeze_moves_to_frozen() {
        let mut bm = BalanceManager::new();
        let user = UserId::new();
        bm.deposit(user, "USDT", Decimal::new(1000, 0));
        bm.freeze(user, "USDT", Decimal::new(400, 0)).unwrap();
        let bal = bm.balance(user, "USDT");
        assert_eq!(bal.available, Decimal::new(600, 0));
        assert_eq!(bal.frozen, Decimal::new(400, 0));
    }

    #[test]
    fn freeze_insufficient_fails() {
        let mut bm = BalanceManager::new();
        let user = UserId::new();
        bm.deposit(user, "USDT", Decimal::new(100, 0));
        let err = bm.freeze(user, "USDT", Decimal::new(200, 0)).unwrap_err();
        assert!(matches!(err, OpenmatchError::InsufficientBalance { .. }));
        // Balance unchanged
        let bal = bm.balance(user, "USDT");
        assert_eq!(bal.available, Decimal::new(100, 0));
    }

    #[test]
    fn unfreeze_restores_available() {
        let mut bm = BalanceManager::new();
        let user = UserId::new();
        bm.deposit(user, "USDT", Decimal::new(1000, 0));
        bm.freeze(user, "USDT", Decimal::new(400, 0)).unwrap();
        bm.unfreeze(user, "USDT", Decimal::new(400, 0)).unwrap();
        let bal = bm.balance(user, "USDT");
        assert_eq!(bal.available, Decimal::new(1000, 0));
        assert_eq!(bal.frozen, Decimal::ZERO);
    }

    #[test]
    fn consume_frozen_reduces_frozen() {
        let mut bm = BalanceManager::new();
        let user = UserId::new();
        bm.deposit(user, "USDT", Decimal::new(1000, 0));
        bm.freeze(user, "USDT", Decimal::new(500, 0)).unwrap();
        bm.consume_frozen(user, "USDT", Decimal::new(500, 0))
            .unwrap();
        let bal = bm.balance(user, "USDT");
        assert_eq!(bal.available, Decimal::new(500, 0));
        assert_eq!(bal.frozen, Decimal::ZERO);
    }

    #[test]
    fn credit_adds_to_available() {
        let mut bm = BalanceManager::new();
        let user = UserId::new();
        bm.credit(user, "BTC", Decimal::ONE);
        let bal = bm.balance(user, "BTC");
        assert_eq!(bal.available, Decimal::ONE);
    }

    #[test]
    fn total_supply_sums_all_users() {
        let mut bm = BalanceManager::new();
        let u1 = UserId::new();
        let u2 = UserId::new();
        bm.deposit(u1, "USDT", Decimal::new(1000, 0));
        bm.deposit(u2, "USDT", Decimal::new(500, 0));
        bm.freeze(u1, "USDT", Decimal::new(300, 0)).unwrap();
        assert_eq!(bm.total_supply("USDT"), Decimal::new(1500, 0));
    }

    #[test]
    fn nonexistent_balance_is_zero() {
        let bm = BalanceManager::new();
        let bal = bm.balance(UserId::new(), "BTC");
        assert!(bal.is_zero());
    }
}
