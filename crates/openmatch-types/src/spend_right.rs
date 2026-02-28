//! # SpendRight — the cryptographic pre-commitment primitive
//!
//! A `SpendRight` (SR) replaces the old `FreezeProof`. It is a **spendable
//! reservation token** minted atomically when funds are frozen.
//!
//! ## State Machine
//!
//! ```text
//!   ┌────────┐  settlement   ┌───────┐
//!   │ ACTIVE ├──────────────▶│ SPENT │
//!   └───┬────┘               └───────┘
//!       │ cancel/expire
//!       ▼
//!   ┌──────────┐
//!   │ RELEASED │
//!   └──────────┘
//! ```
//!
//! ## Security Properties
//!
//! - **Atomic minting**: SR is created only when balance freeze succeeds
//! - **Single-use**: ACTIVE → SPENT is irreversible, prevents double-spend
//! - **Nonce-bound**: each SR has a unique nonce, preventing replay
//! - **Signature-bound**: signed by issuing node's ed25519 key
//! - **Time-bound**: expires after epoch window, preventing stale orders

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{EpochId, NodeId, OrderId, SpendRightId, UserId};

/// The lifecycle state of a SpendRight.
///
/// Transitions are **monotonic** (never go backwards):
/// - `Active → Spent` (settlement consumed the SR)
/// - `Active → Released` (order cancelled or SR expired)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SpendRightState {
    /// Funds are frozen. This SR can be used for matching.
    Active,
    /// Settlement consumed this SR. Funds have been transferred.
    /// **Irreversible.** This is what prevents double-spend.
    Spent,
    /// The order was cancelled or the SR expired. Funds unfrozen.
    Released,
}

impl SpendRightState {
    /// Can this SR transition to the given target state?
    #[must_use]
    pub fn can_transition_to(&self, target: Self) -> bool {
        matches!((self, target), (Self::Active, Self::Spent | Self::Released))
    }
}

impl std::fmt::Display for SpendRightState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "ACTIVE"),
            Self::Spent => write!(f, "SPENT"),
            Self::Released => write!(f, "RELEASED"),
        }
    }
}

/// A SpendRight: cryptographic proof that funds are frozen for a specific order.
///
/// Orders entering MatchCore reference an `sr_id`. The Security Envelope
/// mints SRs; the Finality Plane consumes them.
///
/// MatchCore **never** sees the full SpendRight — only the `sr_id` reference
/// on each Order. This keeps MatchCore purely computational.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendRight {
    /// Globally unique SR identifier.
    pub id: SpendRightId,
    /// The order this SR funds.
    pub order_id: OrderId,
    /// The user whose balance was frozen.
    pub user_id: UserId,
    /// The asset that was frozen (e.g., "USDT" for a buy, "BTC" for a sell).
    pub asset: String,
    /// Amount frozen.
    pub amount: Decimal,
    /// The node that issued this SR (and signed it).
    pub issuer_node: NodeId,
    /// Current lifecycle state.
    pub state: SpendRightState,
    /// Ed25519 signature over the canonical signing payload.
    pub signature: Vec<u8>,
    /// Unique nonce to prevent replay attacks.
    pub nonce: u64,
    /// The epoch this SR was minted for.
    pub epoch_id: EpochId,
    /// When the SR was minted.
    pub created_at: DateTime<Utc>,
    /// When the SR expires (order must match before this).
    pub expires_at: DateTime<Utc>,
}

impl SpendRight {
    /// Canonical signing payload for ed25519 verification.
    ///
    /// Format: `"openmatch:sr:v1:" || sr_id || order_id || user_id || asset || amount || nonce || epoch_id`
    #[must_use]
    pub fn signing_payload(&self) -> Vec<u8> {
        let mut payload = Vec::with_capacity(256);
        payload.extend_from_slice(b"openmatch:sr:v1:");
        payload.extend_from_slice(self.id.0.as_bytes());
        payload.extend_from_slice(self.order_id.0.as_bytes());
        payload.extend_from_slice(self.user_id.0.as_bytes());
        payload.extend_from_slice(self.asset.as_bytes());
        payload.extend_from_slice(self.amount.to_string().as_bytes());
        payload.extend_from_slice(&self.nonce.to_le_bytes());
        payload.extend_from_slice(&self.epoch_id.0.to_le_bytes());
        payload
    }

    /// Returns `true` if this SR has expired.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    /// Returns `true` if this SR is currently usable for matching.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.state == SpendRightState::Active && !self.is_expired()
    }

    /// Attempt to transition to SPENT state.
    ///
    /// # Errors
    /// Returns error if current state is not Active.
    pub fn mark_spent(&mut self) -> crate::Result<()> {
        if !self.state.can_transition_to(SpendRightState::Spent) {
            return Err(crate::OpenmatchError::InvalidSpendRight {
                reason: format!(
                    "Cannot transition SR {} from {} to SPENT",
                    self.id, self.state
                ),
            });
        }
        self.state = SpendRightState::Spent;
        Ok(())
    }

    /// Attempt to transition to RELEASED state.
    ///
    /// # Errors
    /// Returns error if current state is not Active.
    pub fn mark_released(&mut self) -> crate::Result<()> {
        if !self.state.can_transition_to(SpendRightState::Released) {
            return Err(crate::OpenmatchError::InvalidSpendRight {
                reason: format!(
                    "Cannot transition SR {} from {} to RELEASED",
                    self.id, self.state
                ),
            });
        }
        self.state = SpendRightState::Released;
        Ok(())
    }
}

/// Dummy SpendRight for testing. **Never use in production.**
#[cfg(any(test, feature = "test-helpers"))]
impl SpendRight {
    /// Create a dummy SR for unit tests.
    pub fn dummy(
        order_id: OrderId,
        user_id: UserId,
        asset: &str,
        amount: Decimal,
        epoch_id: EpochId,
    ) -> Self {
        Self {
            id: SpendRightId::new(),
            order_id,
            user_id,
            asset: asset.to_string(),
            amount,
            issuer_node: NodeId([0u8; 32]),
            state: SpendRightState::Active,
            signature: vec![0u8; 64],
            nonce: rand::random::<u64>(),
            epoch_id,
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sr() -> SpendRight {
        SpendRight::dummy(
            OrderId::new(),
            UserId::new(),
            "USDT",
            Decimal::new(10000, 2),
            EpochId(1),
        )
    }

    #[test]
    fn state_transitions_valid() {
        assert!(SpendRightState::Active.can_transition_to(SpendRightState::Spent));
        assert!(SpendRightState::Active.can_transition_to(SpendRightState::Released));
    }

    #[test]
    fn state_transitions_invalid() {
        assert!(!SpendRightState::Spent.can_transition_to(SpendRightState::Active));
        assert!(!SpendRightState::Spent.can_transition_to(SpendRightState::Released));
        assert!(!SpendRightState::Released.can_transition_to(SpendRightState::Active));
        assert!(!SpendRightState::Released.can_transition_to(SpendRightState::Spent));
    }

    #[test]
    fn mark_spent_from_active() {
        let mut sr = make_sr();
        assert!(sr.mark_spent().is_ok());
        assert_eq!(sr.state, SpendRightState::Spent);
    }

    #[test]
    fn double_spend_blocked() {
        let mut sr = make_sr();
        sr.mark_spent().unwrap();
        assert!(sr.mark_spent().is_err(), "SPENT → SPENT must fail");
    }

    #[test]
    fn mark_released_from_active() {
        let mut sr = make_sr();
        assert!(sr.mark_released().is_ok());
        assert_eq!(sr.state, SpendRightState::Released);
    }

    #[test]
    fn released_cannot_be_spent() {
        let mut sr = make_sr();
        sr.mark_released().unwrap();
        assert!(sr.mark_spent().is_err(), "RELEASED → SPENT must fail");
    }

    #[test]
    fn signing_payload_deterministic() {
        let sr = make_sr();
        assert_eq!(sr.signing_payload(), sr.signing_payload());
    }

    #[test]
    fn signing_payload_differs_by_nonce() {
        let mut sr1 = make_sr();
        sr1.nonce = 1;
        let mut sr2 = sr1.clone();
        sr2.nonce = 2;
        assert_ne!(sr1.signing_payload(), sr2.signing_payload());
    }

    #[test]
    fn is_active_when_not_expired() {
        let sr = make_sr();
        assert!(sr.is_active());
    }

    #[test]
    fn serde_roundtrip() {
        let sr = make_sr();
        let json = serde_json::to_string(&sr).unwrap();
        let back: SpendRight = serde_json::from_str(&json).unwrap();
        assert_eq!(sr.id, back.id);
        assert_eq!(sr.amount, back.amount);
        assert_eq!(sr.state, back.state);
    }
}
