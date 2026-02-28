//! Epoch lifecycle types for the OpenMatch batch auction model.
//!
//! Each epoch cycles through four non-overlapping phases:
//! **COLLECT → SEAL → MATCH → FINALIZE**
//!
//! During COLLECT, orders flow into the pending buffer.
//! During SEAL, the buffer is sealed and the SealedBatch + BatchDigest are produced.
//! During MATCH, deterministic batch matching runs on the sealed input.
//! During FINALIZE, trades are settled via the 3-tier settlement engine and
//! SpendRights are consumed (ACTIVE → SPENT).

use std::{fmt, time::Duration};

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{EpochId, NodeId, Order, Trade, constants};

/// The four non-overlapping phases of an epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EpochPhase {
    /// Accepting new orders into the pending buffer.
    Collect,
    /// Pending buffer sealed; producing SealedBatch and exchanging BatchDigests.
    Seal,
    /// Running deterministic batch matching on the sealed input.
    Match,
    /// Trades produced; executing 3-tier settlement and consuming SpendRights.
    Finalize,
}

impl fmt::Display for EpochPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Collect => write!(f, "COLLECT"),
            Self::Seal => write!(f, "SEAL"),
            Self::Match => write!(f, "MATCH"),
            Self::Finalize => write!(f, "FINALIZE"),
        }
    }
}

impl EpochPhase {
    /// Return the next phase in the cycle.
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Collect => Self::Seal,
            Self::Seal => Self::Match,
            Self::Match => Self::Finalize,
            Self::Finalize => Self::Collect,
        }
    }
}

// ---------------------------------------------------------------------------
// SealedBatch — the immutable input to MatchCore
// ---------------------------------------------------------------------------

/// A sealed batch of orders ready for deterministic matching.
///
/// Once sealed, the batch is immutable: its `batch_hash` commits to the
/// exact set of orders. Every node that receives the same `SealedBatch`
/// will produce the same `TradeBundle`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedBatch {
    /// The epoch this batch belongs to.
    pub epoch_id: EpochId,
    /// The orders in deterministic order (sorted by sequence).
    pub orders: Vec<Order>,
    /// SHA-256 hash committing to the ordered set of orders.
    pub batch_hash: [u8; 32],
    /// When this batch was sealed.
    pub sealed_at: DateTime<Utc>,
    /// The node that sealed this batch.
    pub sealer_node: NodeId,
}

// ---------------------------------------------------------------------------
// TradeBundle — the deterministic output from MatchCore
// ---------------------------------------------------------------------------

/// The deterministic output of the matching engine for one epoch.
///
/// Given the same `SealedBatch`, every node produces the exact same
/// `TradeBundle` — same trades, same trade_root, same clearing price.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeBundle {
    /// The epoch that produced these trades.
    pub epoch_id: EpochId,
    /// The trades produced by matching.
    pub trades: Vec<Trade>,
    /// Merkle root hash over all trades (for cross-node verification).
    pub trade_root: [u8; 32],
    /// The input hash (from `SealedBatch::batch_hash`) for traceability.
    pub input_hash: [u8; 32],
    /// The uniform clearing price used, if any.
    pub clearing_price: Option<Decimal>,
    /// Orders that remain unmatched (partially filled or no crossing).
    pub remaining_orders: Vec<Order>,
}

// ---------------------------------------------------------------------------
// BatchDigest — lightweight attestation of a sealed batch
// ---------------------------------------------------------------------------

/// Lightweight cryptographic attestation of a sealed batch.
///
/// Contains only the metadata and hash, not the full order set.
/// Nodes exchange `BatchDigest` to verify they sealed the same batch
/// without transmitting all orders.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchDigest {
    /// The epoch this digest belongs to.
    pub epoch_id: EpochId,
    /// SHA-256 hash of the sealed batch (must match `SealedBatch::batch_hash`).
    pub batch_hash: [u8; 32],
    /// Number of orders in the batch.
    pub order_count: usize,
    /// The node that signed this digest.
    pub signer_node: NodeId,
    /// Ed25519 signature over (epoch_id || batch_hash || order_count).
    pub signature: Vec<u8>,
}

/// Configuration for epoch timing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpochConfig {
    /// Duration of the COLLECT phase.
    pub collect_duration: Duration,
    /// Duration of the SEAL phase.
    pub seal_duration: Duration,
    /// Maximum duration for the MATCH phase (hard timeout).
    pub match_timeout: Duration,
    /// Maximum duration for the FINALIZE phase (hard timeout).
    pub finalize_timeout: Duration,
    /// Grace period after COLLECT ends before sealing the buffer.
    /// Late-arriving orders within this window are still accepted.
    pub seal_grace: Duration,
}

impl Default for EpochConfig {
    fn default() -> Self {
        Self {
            collect_duration: Duration::from_millis(constants::DEFAULT_COLLECT_MS),
            seal_duration: Duration::from_millis(constants::DEFAULT_SEAL_MS),
            match_timeout: Duration::from_millis(constants::DEFAULT_MATCH_TIMEOUT_MS),
            finalize_timeout: Duration::from_millis(constants::DEFAULT_FINALIZE_TIMEOUT_MS),
            seal_grace: Duration::from_millis(constants::DEFAULT_SEAL_GRACE_MS),
        }
    }
}

impl EpochConfig {
    /// Total duration of one epoch cycle (all four phases).
    #[must_use]
    pub fn total_duration(&self) -> Duration {
        self.collect_duration + self.seal_duration + self.match_timeout + self.finalize_timeout
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EpochId;

    #[test]
    fn epoch_phase_cycle() {
        assert_eq!(EpochPhase::Collect.next(), EpochPhase::Seal);
        assert_eq!(EpochPhase::Seal.next(), EpochPhase::Match);
        assert_eq!(EpochPhase::Match.next(), EpochPhase::Finalize);
        assert_eq!(EpochPhase::Finalize.next(), EpochPhase::Collect);
    }

    #[test]
    fn epoch_phase_display() {
        assert_eq!(format!("{}", EpochPhase::Collect), "COLLECT");
        assert_eq!(format!("{}", EpochPhase::Seal), "SEAL");
        assert_eq!(format!("{}", EpochPhase::Match), "MATCH");
        assert_eq!(format!("{}", EpochPhase::Finalize), "FINALIZE");
    }

    #[test]
    fn epoch_id_next() {
        assert_eq!(EpochId(0).next(), EpochId(1));
        assert_eq!(EpochId(99).next(), EpochId(100));
    }

    #[test]
    fn epoch_config_default() {
        let cfg = EpochConfig::default();
        assert_eq!(cfg.collect_duration.as_millis(), 1000);
        assert_eq!(cfg.seal_duration.as_millis(), 200);
        assert_eq!(cfg.match_timeout.as_millis(), 500);
        assert_eq!(cfg.finalize_timeout.as_millis(), 2000);
        assert_eq!(cfg.seal_grace.as_millis(), 50);
    }

    #[test]
    fn epoch_config_total_duration() {
        let cfg = EpochConfig::default();
        // 1000 + 200 + 500 + 2000 = 3700ms
        assert_eq!(cfg.total_duration().as_millis(), 3700);
    }

    #[test]
    fn epoch_phase_serde_roundtrip() {
        let phase = EpochPhase::Match;
        let json = serde_json::to_string(&phase).unwrap();
        let back: EpochPhase = serde_json::from_str(&json).unwrap();
        assert_eq!(phase, back);
    }

    #[test]
    fn seal_phase_serde_roundtrip() {
        let phase = EpochPhase::Seal;
        let json = serde_json::to_string(&phase).unwrap();
        let back: EpochPhase = serde_json::from_str(&json).unwrap();
        assert_eq!(phase, back);
    }
}
