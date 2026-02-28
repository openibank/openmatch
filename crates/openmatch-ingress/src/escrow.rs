//! Escrow manager — mints and releases SpendRights.
//!
//! The EscrowManager atomically freezes funds and mints a SpendRight.
//! When an order is cancelled or a SR expires, it releases the funds
//! by unfreezing them and marking the SR as RELEASED.

use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};

use chrono::Utc;
use openmatch_types::{
    EpochId, NodeId, OpenmatchError, OrderId, Result, SpendRight, SpendRightId, SpendRightState,
    UserId,
};
use rust_decimal::Decimal;

use crate::balance_manager::BalanceManager;

/// Monotonic nonce counter for SpendRight minting.
static NONCE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Manages the SpendRight lifecycle: minting, releasing, and lookup.
pub struct EscrowManager {
    /// All SpendRights indexed by their ID.
    spend_rights: HashMap<SpendRightId, SpendRight>,
    /// The node identity for signing SRs.
    node_id: NodeId,
}

impl EscrowManager {
    /// Create a new escrow manager for the given node.
    #[must_use]
    pub fn new(node_id: NodeId) -> Self {
        Self {
            spend_rights: HashMap::new(),
            node_id,
        }
    }

    /// Atomically freeze funds and mint a SpendRight.
    ///
    /// 1. Freeze `amount` of `asset` from the user's balance
    /// 2. Create a new SpendRight in ACTIVE state
    /// 3. Return the SR ID
    ///
    /// If the freeze fails (insufficient balance), no SR is minted.
    ///
    /// # Errors
    /// Returns `InsufficientBalance` if the user doesn't have enough funds.
    pub fn mint(
        &mut self,
        balance_manager: &mut BalanceManager,
        order_id: OrderId,
        user_id: UserId,
        asset: &str,
        amount: Decimal,
        epoch_id: EpochId,
    ) -> Result<SpendRightId> {
        // Step 1: Freeze funds (atomic — if this fails, nothing changes)
        balance_manager.freeze(user_id, asset, amount)?;

        // Step 2: Create the SpendRight
        let sr_id = SpendRightId::new();
        let now = Utc::now();
        let sr = SpendRight {
            id: sr_id,
            order_id,
            user_id,
            asset: asset.to_string(),
            amount,
            issuer_node: self.node_id,
            state: SpendRightState::Active,
            signature: vec![0u8; 64], // Placeholder — real impl uses ed25519
            nonce: NONCE_COUNTER.fetch_add(1, Ordering::Relaxed),
            epoch_id,
            created_at: now,
            expires_at: now + chrono::Duration::hours(1),
        };

        // Step 3: Store and return
        self.spend_rights.insert(sr_id, sr);
        Ok(sr_id)
    }

    /// Release a SpendRight (cancel or expire). Unfreezes the funds.
    ///
    /// # Errors
    /// - `InvalidSpendRight` if the SR doesn't exist or isn't ACTIVE
    /// - `InsufficientFrozen` if the unfreeze fails
    pub fn release(
        &mut self,
        balance_manager: &mut BalanceManager,
        sr_id: SpendRightId,
    ) -> Result<()> {
        let sr =
            self.spend_rights
                .get_mut(&sr_id)
                .ok_or_else(|| OpenmatchError::InvalidSpendRight {
                    reason: format!("SpendRight {sr_id} not found"),
                })?;

        if sr.state != SpendRightState::Active {
            return Err(OpenmatchError::InvalidSpendRight {
                reason: format!("SpendRight {sr_id} is {}, not ACTIVE", sr.state),
            });
        }

        // Unfreeze the funds
        balance_manager.unfreeze(sr.user_id, &sr.asset, sr.amount)?;

        // Mark SR as released
        sr.mark_released()?;
        Ok(())
    }

    /// Mark a SpendRight as SPENT (called during settlement).
    ///
    /// Note: This does NOT unfreeze funds — the settlement engine
    /// handles the actual balance transfer.
    ///
    /// # Errors
    /// Returns `InvalidSpendRight` if the SR doesn't exist or isn't ACTIVE.
    pub fn mark_spent(&mut self, sr_id: SpendRightId) -> Result<()> {
        let sr =
            self.spend_rights
                .get_mut(&sr_id)
                .ok_or_else(|| OpenmatchError::InvalidSpendRight {
                    reason: format!("SpendRight {sr_id} not found"),
                })?;

        sr.mark_spent()
    }

    /// Look up a SpendRight by ID.
    #[must_use]
    pub fn get(&self, sr_id: &SpendRightId) -> Option<&SpendRight> {
        self.spend_rights.get(sr_id)
    }

    /// Check if a SpendRight is currently active.
    #[must_use]
    pub fn is_active(&self, sr_id: &SpendRightId) -> bool {
        self.spend_rights
            .get(sr_id)
            .is_some_and(SpendRight::is_active)
    }

    /// Number of SpendRights tracked.
    #[must_use]
    pub fn count(&self) -> usize {
        self.spend_rights.len()
    }

    /// Number of ACTIVE SpendRights.
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.spend_rights
            .values()
            .filter(|sr| sr.state == SpendRightState::Active)
            .count()
    }

    /// The node ID this escrow manager operates on behalf of.
    #[must_use]
    pub fn node_id(&self) -> NodeId {
        self.node_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (EscrowManager, BalanceManager) {
        let em = EscrowManager::new(NodeId([0u8; 32]));
        let bm = BalanceManager::new();
        (em, bm)
    }

    #[test]
    fn mint_freezes_and_creates_sr() {
        let (mut em, mut bm) = setup();
        let user = UserId::new();
        bm.deposit(user, "USDT", Decimal::new(10000, 0));

        let sr_id = em
            .mint(
                &mut bm,
                OrderId::new(),
                user,
                "USDT",
                Decimal::new(5000, 0),
                EpochId(1),
            )
            .unwrap();

        // Balance should be frozen
        let bal = bm.balance(user, "USDT");
        assert_eq!(bal.available, Decimal::new(5000, 0));
        assert_eq!(bal.frozen, Decimal::new(5000, 0));

        // SR should exist and be active
        assert!(em.is_active(&sr_id));
        assert_eq!(em.count(), 1);
        assert_eq!(em.active_count(), 1);
    }

    #[test]
    fn mint_fails_insufficient_balance() {
        let (mut em, mut bm) = setup();
        let user = UserId::new();
        bm.deposit(user, "USDT", Decimal::new(100, 0));

        let err = em
            .mint(
                &mut bm,
                OrderId::new(),
                user,
                "USDT",
                Decimal::new(200, 0),
                EpochId(1),
            )
            .unwrap_err();
        assert!(matches!(err, OpenmatchError::InsufficientBalance { .. }));

        // No SR should be created
        assert_eq!(em.count(), 0);
        // Balance unchanged
        assert_eq!(bm.balance(user, "USDT").available, Decimal::new(100, 0));
    }

    #[test]
    fn release_unfreezes_and_marks_released() {
        let (mut em, mut bm) = setup();
        let user = UserId::new();
        bm.deposit(user, "USDT", Decimal::new(10000, 0));

        let sr_id = em
            .mint(
                &mut bm,
                OrderId::new(),
                user,
                "USDT",
                Decimal::new(5000, 0),
                EpochId(1),
            )
            .unwrap();

        em.release(&mut bm, sr_id).unwrap();

        // Balance should be fully available again
        let bal = bm.balance(user, "USDT");
        assert_eq!(bal.available, Decimal::new(10000, 0));
        assert_eq!(bal.frozen, Decimal::ZERO);

        // SR should be RELEASED
        assert!(!em.is_active(&sr_id));
        let sr = em.get(&sr_id).unwrap();
        assert_eq!(sr.state, SpendRightState::Released);
    }

    #[test]
    fn double_release_fails() {
        let (mut em, mut bm) = setup();
        let user = UserId::new();
        bm.deposit(user, "USDT", Decimal::new(10000, 0));

        let sr_id = em
            .mint(
                &mut bm,
                OrderId::new(),
                user,
                "USDT",
                Decimal::new(5000, 0),
                EpochId(1),
            )
            .unwrap();

        em.release(&mut bm, sr_id).unwrap();
        let err = em.release(&mut bm, sr_id).unwrap_err();
        assert!(matches!(err, OpenmatchError::InvalidSpendRight { .. }));
    }

    #[test]
    fn mark_spent_transitions_state() {
        let (mut em, mut bm) = setup();
        let user = UserId::new();
        bm.deposit(user, "USDT", Decimal::new(10000, 0));

        let sr_id = em
            .mint(
                &mut bm,
                OrderId::new(),
                user,
                "USDT",
                Decimal::new(5000, 0),
                EpochId(1),
            )
            .unwrap();

        em.mark_spent(sr_id).unwrap();

        let sr = em.get(&sr_id).unwrap();
        assert_eq!(sr.state, SpendRightState::Spent);
        assert_eq!(em.active_count(), 0);
    }

    #[test]
    fn spent_cannot_be_released() {
        let (mut em, mut bm) = setup();
        let user = UserId::new();
        bm.deposit(user, "USDT", Decimal::new(10000, 0));

        let sr_id = em
            .mint(
                &mut bm,
                OrderId::new(),
                user,
                "USDT",
                Decimal::new(5000, 0),
                EpochId(1),
            )
            .unwrap();

        em.mark_spent(sr_id).unwrap();
        let err = em.release(&mut bm, sr_id).unwrap_err();
        assert!(matches!(err, OpenmatchError::InvalidSpendRight { .. }));
    }

    #[test]
    fn nonexistent_sr_errors() {
        let (mut em, mut bm) = setup();
        let fake_id = SpendRightId::new();
        let err = em.release(&mut bm, fake_id).unwrap_err();
        assert!(matches!(err, OpenmatchError::InvalidSpendRight { .. }));
    }
}
