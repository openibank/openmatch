//! Supply conservation invariant checker.
//!
//! Mathematical invariant enforced after every settlement:
//! ```text
//! ∀ asset: Σ(available + frozen) == Σ(deposits) - Σ(withdrawals)
//! ```
//!
//! If this invariant ever breaks, the system halts with a critical alert.
//! This is the ultimate safety net — if supply is not conserved, something
//! has gone catastrophically wrong.

use std::collections::HashMap;

use openmatch_types::{Asset, OpenmatchError, Result};
use rust_decimal::Decimal;

/// Tracks per-asset supply totals and validates conservation after every
/// settlement cycle.
pub struct SupplyConservation {
    /// Total deposits per asset since genesis.
    deposits: HashMap<Asset, Decimal>,
    /// Total withdrawals per asset since genesis.
    withdrawals: HashMap<Asset, Decimal>,
}

impl SupplyConservation {
    /// Create a new supply conservation tracker.
    #[must_use]
    pub fn new() -> Self {
        Self {
            deposits: HashMap::new(),
            withdrawals: HashMap::new(),
        }
    }

    /// Record a deposit.
    pub fn record_deposit(&mut self, asset: &str, amount: Decimal) {
        *self
            .deposits
            .entry(asset.to_string())
            .or_insert(Decimal::ZERO) += amount;
    }

    /// Record a withdrawal.
    pub fn record_withdrawal(&mut self, asset: &str, amount: Decimal) {
        *self
            .withdrawals
            .entry(asset.to_string())
            .or_insert(Decimal::ZERO) += amount;
    }

    /// Expected total supply for an asset: deposits - withdrawals.
    #[must_use]
    pub fn expected_supply(&self, asset: &str) -> Decimal {
        let deposited = self.deposits.get(asset).copied().unwrap_or(Decimal::ZERO);
        let withdrawn = self
            .withdrawals
            .get(asset)
            .copied()
            .unwrap_or(Decimal::ZERO);
        deposited - withdrawn
    }

    /// Verify that the actual supply (sum of all user balances) matches
    /// the expected supply (deposits - withdrawals) for a given asset.
    ///
    /// # Errors
    /// Returns [`OpenmatchError::SupplyInvariantViolation`] if actual ≠ expected.
    pub fn verify(&self, asset: &str, actual_supply: Decimal) -> Result<()> {
        let expected = self.expected_supply(asset);
        if actual_supply != expected {
            return Err(OpenmatchError::SupplyInvariantViolation {
                reason: format!(
                    "Asset {asset}: actual supply {actual_supply} != expected {expected} \
                     (deposits={}, withdrawals={})",
                    self.deposits.get(asset).copied().unwrap_or(Decimal::ZERO),
                    self.withdrawals
                        .get(asset)
                        .copied()
                        .unwrap_or(Decimal::ZERO),
                ),
            });
        }
        Ok(())
    }

    /// Get all tracked assets.
    #[must_use]
    pub fn tracked_assets(&self) -> Vec<String> {
        let mut assets: std::collections::HashSet<String> = self.deposits.keys().cloned().collect();
        assets.extend(self.withdrawals.keys().cloned());
        assets.into_iter().collect()
    }

    /// Total deposits for an asset.
    #[must_use]
    pub fn total_deposits(&self, asset: &str) -> Decimal {
        self.deposits.get(asset).copied().unwrap_or(Decimal::ZERO)
    }

    /// Total withdrawals for an asset.
    #[must_use]
    pub fn total_withdrawals(&self, asset: &str) -> Decimal {
        self.withdrawals
            .get(asset)
            .copied()
            .unwrap_or(Decimal::ZERO)
    }
}

impl Default for SupplyConservation {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_supply_is_zero() {
        let sc = SupplyConservation::new();
        assert_eq!(sc.expected_supply("BTC"), Decimal::ZERO);
        assert!(sc.verify("BTC", Decimal::ZERO).is_ok());
    }

    #[test]
    fn deposits_increase_expected() {
        let mut sc = SupplyConservation::new();
        sc.record_deposit("USDT", Decimal::new(1000, 0));
        sc.record_deposit("USDT", Decimal::new(500, 0));
        assert_eq!(sc.expected_supply("USDT"), Decimal::new(1500, 0));
    }

    #[test]
    fn withdrawals_decrease_expected() {
        let mut sc = SupplyConservation::new();
        sc.record_deposit("USDT", Decimal::new(1000, 0));
        sc.record_withdrawal("USDT", Decimal::new(300, 0));
        assert_eq!(sc.expected_supply("USDT"), Decimal::new(700, 0));
    }

    #[test]
    fn verify_passes_when_balanced() {
        let mut sc = SupplyConservation::new();
        sc.record_deposit("BTC", Decimal::new(10, 0));
        sc.record_withdrawal("BTC", Decimal::new(3, 0));
        assert!(sc.verify("BTC", Decimal::new(7, 0)).is_ok());
    }

    #[test]
    fn verify_fails_when_imbalanced() {
        let mut sc = SupplyConservation::new();
        sc.record_deposit("BTC", Decimal::new(10, 0));
        let err = sc.verify("BTC", Decimal::new(11, 0)).unwrap_err();
        assert!(matches!(
            err,
            OpenmatchError::SupplyInvariantViolation { .. }
        ));
    }

    #[test]
    fn multiple_assets_independent() {
        let mut sc = SupplyConservation::new();
        sc.record_deposit("BTC", Decimal::new(5, 0));
        sc.record_deposit("USDT", Decimal::new(50000, 0));
        assert_eq!(sc.expected_supply("BTC"), Decimal::new(5, 0));
        assert_eq!(sc.expected_supply("USDT"), Decimal::new(50000, 0));
        assert!(sc.verify("BTC", Decimal::new(5, 0)).is_ok());
        assert!(sc.verify("USDT", Decimal::new(50000, 0)).is_ok());
    }

    #[test]
    fn settlement_does_not_change_supply() {
        // After settlement: funds move between users but total supply is unchanged.
        let mut sc = SupplyConservation::new();
        sc.record_deposit("USDT", Decimal::new(1000, 0));
        sc.record_deposit("BTC", Decimal::new(1, 0));

        // Settlement: buyer gets BTC, seller gets USDT — no deposits/withdrawals.
        // Total supply must remain the same.
        assert!(sc.verify("USDT", Decimal::new(1000, 0)).is_ok());
        assert!(sc.verify("BTC", Decimal::new(1, 0)).is_ok());
    }
}
