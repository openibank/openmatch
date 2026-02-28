//! Risk kernel — hard gate for order validation.
//!
//! The RiskKernel validates every order before it enters the pending buffer.
//! It enforces per-user limits and system-wide safety checks.
//!
//! ## Design Principles
//!
//! - **Fail-closed**: If any check errors, the order is rejected
//! - **No bypass**: Every order path goes through the kernel
//! - **Pluggable**: Enterprise risk logic can tighten (never weaken) rules
//! - **Zero latency impact on MatchCore**: All risk checks happen in ingress

use std::collections::HashMap;

use openmatch_types::{EpochId, OpenmatchError, Order, OrderType, Result, UserId};
use rust_decimal::Decimal;

/// Hard risk gate that validates orders before they enter the pending buffer.
pub struct RiskKernel {
    /// Maximum orders per user per epoch.
    max_orders_per_user_per_epoch: usize,
    /// Maximum single order size (base asset).
    max_order_size: Decimal,
    /// Maximum price deviation from last known price (multiplier).
    max_price_deviation: Decimal,
    /// Per-user order count for the current epoch.
    epoch_order_counts: HashMap<UserId, usize>,
    /// Current epoch.
    current_epoch: EpochId,
    /// Last known prices per market (for price sanity checks).
    last_prices: HashMap<String, Decimal>,
}

impl RiskKernel {
    /// Create a new risk kernel with default limits.
    #[must_use]
    pub fn new() -> Self {
        Self {
            max_orders_per_user_per_epoch: 50,
            max_order_size: Decimal::new(100, 0), // 100 base units
            max_price_deviation: Decimal::new(10, 0), // 10x deviation
            epoch_order_counts: HashMap::new(),
            current_epoch: EpochId(0),
            last_prices: HashMap::new(),
        }
    }

    /// Create a risk kernel with custom limits.
    #[must_use]
    pub fn with_limits(
        max_orders_per_user_per_epoch: usize,
        max_order_size: Decimal,
        max_price_deviation: Decimal,
    ) -> Self {
        Self {
            max_orders_per_user_per_epoch,
            max_order_size,
            max_price_deviation,
            epoch_order_counts: HashMap::new(),
            current_epoch: EpochId(0),
            last_prices: HashMap::new(),
        }
    }

    /// Advance to a new epoch. Resets per-epoch counters.
    pub fn advance_epoch(&mut self, epoch_id: EpochId) {
        self.current_epoch = epoch_id;
        self.epoch_order_counts.clear();
    }

    /// Update the last known price for a market.
    pub fn set_last_price(&mut self, market: &str, price: Decimal) {
        self.last_prices.insert(market.to_string(), price);
    }

    /// Validate an order against all risk checks.
    ///
    /// # Errors
    /// Returns specific error for each check that fails.
    pub fn validate(&mut self, order: &Order) -> Result<()> {
        // 1. Basic validation
        if order.quantity.is_zero() || order.quantity.is_sign_negative() {
            return Err(OpenmatchError::InvalidOrder {
                reason: "Quantity must be positive".to_string(),
            });
        }

        // 2. Cancel orders bypass most checks
        if order.order_type == OrderType::Cancel {
            return Ok(());
        }

        // 3. Order size check
        if order.quantity > self.max_order_size {
            return Err(OpenmatchError::InvalidOrder {
                reason: format!(
                    "Order size {} exceeds maximum {}",
                    order.quantity, self.max_order_size,
                ),
            });
        }

        // 4. Price sanity check (for limit orders)
        if order.order_type == OrderType::Limit {
            if let Some(price) = order.price {
                if price.is_zero() || price.is_sign_negative() {
                    return Err(OpenmatchError::SuspiciousPrice {
                        reason: "Price must be positive".to_string(),
                    });
                }
                self.check_price_deviation(&order.market.symbol(), price)?;
            }
        }

        // 5. Per-user epoch rate limit
        let count = self.epoch_order_counts.entry(order.user_id).or_insert(0);
        if *count >= self.max_orders_per_user_per_epoch {
            return Err(OpenmatchError::OrderFloodDetected {
                count: *count,
                window_ms: 0, // epoch-based, not time-based
            });
        }
        *count += 1;

        Ok(())
    }

    /// Check if a price deviates too far from the last known price.
    fn check_price_deviation(&self, market: &str, price: Decimal) -> Result<()> {
        if let Some(last_price) = self.last_prices.get(market) {
            if !last_price.is_zero() {
                let ratio = if price > *last_price {
                    price / *last_price
                } else {
                    *last_price / price
                };
                if ratio > self.max_price_deviation {
                    return Err(OpenmatchError::SuspiciousPrice {
                        reason: format!(
                            "Price {price} deviates {ratio}x from last known {last_price} \
                             (max {max}x)",
                            max = self.max_price_deviation,
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    /// Get the order count for a user in the current epoch.
    #[must_use]
    pub fn user_order_count(&self, user_id: &UserId) -> usize {
        self.epoch_order_counts.get(user_id).copied().unwrap_or(0)
    }
}

impl Default for RiskKernel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use openmatch_types::*;

    use super::*;

    fn make_buy(price: Decimal, qty: Decimal) -> Order {
        Order::dummy_limit(OrderSide::Buy, price, qty)
    }

    #[test]
    fn valid_order_passes() {
        let mut rk = RiskKernel::new();
        let order = make_buy(Decimal::new(100, 0), Decimal::ONE);
        assert!(rk.validate(&order).is_ok());
    }

    #[test]
    fn zero_quantity_rejected() {
        let mut rk = RiskKernel::new();
        let mut order = make_buy(Decimal::new(100, 0), Decimal::ZERO);
        order.quantity = Decimal::ZERO;
        order.remaining_qty = Decimal::ZERO;
        let err = rk.validate(&order).unwrap_err();
        assert!(matches!(err, OpenmatchError::InvalidOrder { .. }));
    }

    #[test]
    fn oversized_order_rejected() {
        let mut rk = RiskKernel::with_limits(50, Decimal::new(10, 0), Decimal::new(10, 0));
        let order = make_buy(Decimal::new(100, 0), Decimal::new(20, 0));
        let err = rk.validate(&order).unwrap_err();
        assert!(matches!(err, OpenmatchError::InvalidOrder { .. }));
    }

    #[test]
    fn suspicious_price_rejected() {
        let mut rk = RiskKernel::new();
        rk.set_last_price("BTC/USDT", Decimal::new(100, 0));

        // 20x deviation should fail (max is 10x)
        let order = make_buy(Decimal::new(2000, 0), Decimal::ONE);
        let err = rk.validate(&order).unwrap_err();
        assert!(matches!(err, OpenmatchError::SuspiciousPrice { .. }));
    }

    #[test]
    fn reasonable_price_passes() {
        let mut rk = RiskKernel::new();
        rk.set_last_price("BTC/USDT", Decimal::new(100, 0));

        // 2x deviation should pass (max is 10x)
        let order = make_buy(Decimal::new(200, 0), Decimal::ONE);
        assert!(rk.validate(&order).is_ok());
    }

    #[test]
    fn epoch_rate_limit() {
        let mut rk = RiskKernel::with_limits(3, Decimal::new(100, 0), Decimal::new(10, 0));
        let user = UserId::new();

        for _ in 0..3 {
            let mut order = make_buy(Decimal::new(100, 0), Decimal::ONE);
            order.user_id = user;
            rk.validate(&order).unwrap();
        }

        // 4th order should be rejected
        let mut order = make_buy(Decimal::new(100, 0), Decimal::ONE);
        order.user_id = user;
        let err = rk.validate(&order).unwrap_err();
        assert!(matches!(err, OpenmatchError::OrderFloodDetected { .. }));
    }

    #[test]
    fn epoch_advance_resets_counts() {
        let mut rk = RiskKernel::with_limits(2, Decimal::new(100, 0), Decimal::new(10, 0));
        let user = UserId::new();

        for _ in 0..2 {
            let mut order = make_buy(Decimal::new(100, 0), Decimal::ONE);
            order.user_id = user;
            rk.validate(&order).unwrap();
        }

        // Advance epoch — counters reset
        rk.advance_epoch(EpochId(1));

        let mut order = make_buy(Decimal::new(100, 0), Decimal::ONE);
        order.user_id = user;
        assert!(rk.validate(&order).is_ok());
    }

    #[test]
    fn cancel_orders_bypass_size_check() {
        let mut rk = RiskKernel::with_limits(50, Decimal::new(1, 0), Decimal::new(10, 0));
        let mut order = make_buy(Decimal::new(100, 0), Decimal::new(999, 0));
        order.order_type = OrderType::Cancel;
        assert!(rk.validate(&order).is_ok());
    }
}
