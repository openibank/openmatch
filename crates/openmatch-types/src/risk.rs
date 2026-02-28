//! Risk management types for agent asset protection.
//!
//! Every trading agent operates within a `RiskLimits` sandbox. The risk gate
//! validates **every** action before execution — there is no path from
//! agent logic to fund movement without passing through risk validation.
//!
//! # Security Model
//!
//! ```text
//! Agent proposes action
//!   → RiskGate.validate(action, context)
//!     → Check exposure ceiling (max frozen across all assets)
//!     → Check per-asset limit
//!     → Check open order count
//!     → Check epoch + daily loss limits
//!     → Check order rate limit
//!     → Check emergency reserve
//!     → IF ALL PASS → Executor.submit(action)
//!     → IF ANY FAIL → Reject + log reason
//! ```

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::UserId;

/// Unique identifier for a trading agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct AgentId(pub uuid::Uuid);

impl AgentId {
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::now_v7())
    }
}

impl Default for AgentId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "agent:{}", self.0)
    }
}

/// Risk limits for a trading agent. Enforced by the RiskGate before any
/// action touches the balance manager or order book.
///
/// # Design Principles
///
/// 1. **Defense in depth**: Multiple independent limits, any one can halt activity
/// 2. **Fail-closed**: If limit check errors, action is rejected (not allowed)
/// 3. **No bypass**: Agents interact through `AgentAction` enum only —
///    no direct BalanceManager access
/// 4. **Auditability**: Every validation decision is logged
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskLimits {
    /// Maximum total frozen balance across all assets, denominated in quote currency.
    /// This is the absolute ceiling on how much capital an agent can lock up.
    ///
    /// Example: `100_000` USDT means the agent can have at most 100K USDT
    /// equivalent frozen across all markets.
    pub max_total_exposure: Decimal,

    /// Maximum frozen balance per single asset.
    /// Prevents concentration risk in one asset.
    pub max_asset_exposure: Decimal,

    /// Maximum number of open (unfilled) orders at any time.
    pub max_open_orders: usize,

    /// Maximum single order size in base asset.
    /// Prevents "fat finger" errors.
    pub max_order_size: Decimal,

    /// Maximum loss per epoch before the agent is **paused**.
    /// Paused agents cannot submit new orders until manually reviewed.
    ///
    /// Loss = frozen_consumed_by_settlement - value_received_by_settlement
    pub max_epoch_loss: Decimal,

    /// Maximum cumulative loss per calendar day before agent is **disabled**.
    /// Disabled agents require admin intervention to re-enable.
    pub max_daily_loss: Decimal,

    /// Minimum available (unfrozen) balance that must remain at all times.
    /// This ensures the user can always withdraw emergency funds even if
    /// the agent has positions open.
    pub min_available_reserve: Decimal,

    /// Maximum orders per second (rate limit).
    pub max_orders_per_second: u32,

    /// Whether this agent is allowed to place market orders.
    /// Market orders can cause unbounded slippage.
    pub allow_market_orders: bool,

    /// Maximum markets this agent can trade simultaneously.
    pub max_markets: usize,
}

impl Default for RiskLimits {
    fn default() -> Self {
        Self {
            max_total_exposure: Decimal::new(10_000, 0), // 10K USDT
            max_asset_exposure: Decimal::new(5_000, 0),  // 5K USDT per asset
            max_open_orders: 50,
            max_order_size: Decimal::new(1, 0), // 1 BTC equivalent
            max_epoch_loss: Decimal::new(500, 0), // 500 USDT per epoch
            max_daily_loss: Decimal::new(2_000, 0), // 2K USDT per day
            min_available_reserve: Decimal::new(1_000, 0), // 1K USDT always available
            max_orders_per_second: 10,
            allow_market_orders: false, // conservative default
            max_markets: 3,
        }
    }
}

/// The result of a risk validation check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskDecision {
    /// Action is approved.
    Approved,
    /// Action is rejected with a reason.
    Rejected { reason: RiskRejectionReason },
    /// Agent is paused (epoch loss limit breached).
    AgentPaused { reason: String },
    /// Agent is disabled (daily loss limit breached). Requires admin.
    AgentDisabled { reason: String },
}

/// Reason for risk gate rejection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskRejectionReason {
    /// Total exposure would exceed `max_total_exposure`.
    ExposureCeilingBreached {
        current: Decimal,
        requested: Decimal,
        limit: Decimal,
    },
    /// Per-asset exposure would exceed `max_asset_exposure`.
    AssetExposureBreached {
        asset: String,
        current: Decimal,
        requested: Decimal,
        limit: Decimal,
    },
    /// Too many open orders.
    OrderCountExceeded { current: usize, limit: usize },
    /// Order size exceeds `max_order_size`.
    OrderTooLarge { size: Decimal, limit: Decimal },
    /// Epoch loss limit breached.
    EpochLossBreached {
        current_loss: Decimal,
        limit: Decimal,
    },
    /// Daily loss limit breached.
    DailyLossBreached {
        current_loss: Decimal,
        limit: Decimal,
    },
    /// Would violate emergency reserve.
    ReserveViolation {
        available_after: Decimal,
        min_reserve: Decimal,
    },
    /// Order rate limit exceeded.
    RateLimitExceeded { orders_this_second: u32, limit: u32 },
    /// Market orders not allowed for this agent.
    MarketOrdersDisabled,
    /// Agent is already paused or disabled.
    AgentNotActive,
    /// Too many markets.
    TooManyMarkets { current: usize, limit: usize },
}

impl std::fmt::Display for RiskRejectionReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ExposureCeilingBreached {
                current,
                requested,
                limit,
            } => {
                write!(
                    f,
                    "Total exposure {current} + {requested} would exceed limit {limit}"
                )
            }
            Self::AssetExposureBreached {
                asset,
                current,
                requested,
                limit,
            } => {
                write!(
                    f,
                    "Asset {asset} exposure {current} + {requested} would exceed limit {limit}"
                )
            }
            Self::OrderCountExceeded { current, limit } => {
                write!(f, "Open order count {current} at limit {limit}")
            }
            Self::OrderTooLarge { size, limit } => {
                write!(f, "Order size {size} exceeds limit {limit}")
            }
            Self::EpochLossBreached {
                current_loss,
                limit,
            } => {
                write!(f, "Epoch loss {current_loss} exceeds limit {limit}")
            }
            Self::DailyLossBreached {
                current_loss,
                limit,
            } => {
                write!(f, "Daily loss {current_loss} exceeds limit {limit}")
            }
            Self::ReserveViolation {
                available_after,
                min_reserve,
            } => {
                write!(
                    f,
                    "Available balance {available_after} would breach reserve {min_reserve}"
                )
            }
            Self::RateLimitExceeded {
                orders_this_second,
                limit,
            } => {
                write!(
                    f,
                    "Rate limit: {orders_this_second} orders/s exceeds {limit}"
                )
            }
            Self::MarketOrdersDisabled => write!(f, "Market orders disabled for this agent"),
            Self::AgentNotActive => write!(f, "Agent is paused or disabled"),
            Self::TooManyMarkets { current, limit } => {
                write!(f, "Trading {current} markets, limit is {limit}")
            }
        }
    }
}

/// Association between an agent and the user account it trades on behalf of.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentBinding {
    /// The agent's unique ID.
    pub agent_id: AgentId,
    /// The user account this agent is authorized to trade for.
    pub user_id: UserId,
    /// Risk limits governing this agent's activity.
    pub limits: RiskLimits,
    /// Whether the agent is currently active.
    pub active: bool,
    /// Human-readable name.
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_risk_limits_are_conservative() {
        let limits = RiskLimits::default();
        assert!(
            !limits.allow_market_orders,
            "Market orders should be disabled by default"
        );
        assert!(limits.max_total_exposure > Decimal::ZERO);
        assert!(limits.min_available_reserve > Decimal::ZERO);
        assert!(limits.max_epoch_loss > Decimal::ZERO);
        assert!(limits.max_daily_loss > limits.max_epoch_loss);
    }

    #[test]
    fn risk_decision_approved() {
        let decision = RiskDecision::Approved;
        assert_eq!(decision, RiskDecision::Approved);
    }

    #[test]
    fn risk_rejection_display() {
        let reason = RiskRejectionReason::ExposureCeilingBreached {
            current: Decimal::new(8000, 0),
            requested: Decimal::new(3000, 0),
            limit: Decimal::new(10000, 0),
        };
        let msg = format!("{reason}");
        assert!(msg.contains("8000"));
        assert!(msg.contains("3000"));
        assert!(msg.contains("10000"));
    }

    #[test]
    fn agent_id_uniqueness() {
        let a = AgentId::new();
        let b = AgentId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn risk_limits_serde_roundtrip() {
        let limits = RiskLimits::default();
        let json = serde_json::to_string(&limits).unwrap();
        let back: RiskLimits = serde_json::from_str(&json).unwrap();
        assert_eq!(limits.max_total_exposure, back.max_total_exposure);
        assert_eq!(limits.allow_market_orders, back.allow_market_orders);
    }

    #[test]
    fn agent_binding_serde_roundtrip() {
        let binding = AgentBinding {
            agent_id: AgentId::new(),
            user_id: UserId::new(),
            limits: RiskLimits::default(),
            active: true,
            name: "TestBot".to_string(),
        };
        let json = serde_json::to_string(&binding).unwrap();
        let back: AgentBinding = serde_json::from_str(&json).unwrap();
        assert_eq!(binding.agent_id, back.agent_id);
        assert_eq!(binding.name, back.name);
    }
}
