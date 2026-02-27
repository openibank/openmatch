//! Balance tracking types for the OpenMatch escrow model.
//!
//! Every user has an `available` balance (usable for new orders)
//! and a `frozen` balance (locked by active orders' escrow).

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// A single balance entry for a (user, asset) pair.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BalanceEntry {
    /// Available for new orders / withdrawal.
    pub available: Decimal,
    /// Frozen / escrowed for active orders awaiting matching or settlement.
    pub frozen: Decimal,
}

impl BalanceEntry {
    /// Create a zero balance.
    #[must_use]
    pub fn new() -> Self {
        Self {
            available: Decimal::ZERO,
            frozen: Decimal::ZERO,
        }
    }

    /// Total balance (available + frozen).
    #[must_use]
    pub fn total(&self) -> Decimal {
        self.available + self.frozen
    }

    /// Whether this entry has no balance at all.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.available.is_zero() && self.frozen.is_zero()
    }
}

impl Default for BalanceEntry {
    fn default() -> Self {
        Self::new()
    }
}

/// Type alias for asset identifiers (e.g., "BTC", "USDT", "ETH").
pub type Asset = String;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balance_entry_default_is_zero() {
        let entry = BalanceEntry::default();
        assert_eq!(entry.available, Decimal::ZERO);
        assert_eq!(entry.frozen, Decimal::ZERO);
        assert!(entry.is_zero());
    }

    #[test]
    fn balance_entry_total() {
        let entry = BalanceEntry {
            available: Decimal::new(100, 0),
            frozen: Decimal::new(50, 0),
        };
        assert_eq!(entry.total(), Decimal::new(150, 0));
        assert!(!entry.is_zero());
    }

    #[test]
    fn balance_entry_serde_roundtrip() {
        let entry = BalanceEntry {
            available: Decimal::new(12345, 2), // 123.45
            frozen: Decimal::new(678, 1),      // 67.8
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: BalanceEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry, back);
    }
}
