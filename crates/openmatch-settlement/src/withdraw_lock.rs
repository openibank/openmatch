//! Phase-aware withdraw lock.
//!
//! Blocks withdrawals during MATCH and FINALIZE phases to prevent
//! balance manipulation while trades are being settled. During COLLECT
//! and SEAL phases, withdrawals are allowed.

use openmatch_types::{EpochPhase, OpenmatchError, Result};

/// Phase-aware lock that blocks withdrawals during critical epoch phases.
///
/// During MATCH and FINALIZE, balances are in flux — allowing withdrawals
/// could break the supply conservation invariant. This lock prevents that.
pub struct WithdrawLock {
    /// The current epoch phase.
    current_phase: EpochPhase,
}

impl WithdrawLock {
    /// Create a new withdraw lock starting in the COLLECT phase.
    #[must_use]
    pub fn new() -> Self {
        Self {
            current_phase: EpochPhase::Collect,
        }
    }

    /// Update the current epoch phase.
    pub fn set_phase(&mut self, phase: EpochPhase) {
        self.current_phase = phase;
    }

    /// Get the current epoch phase.
    #[must_use]
    pub fn current_phase(&self) -> EpochPhase {
        self.current_phase
    }

    /// Check if withdrawals are currently allowed.
    ///
    /// Withdrawals are only permitted during COLLECT and SEAL phases.
    #[must_use]
    pub fn withdrawals_allowed(&self) -> bool {
        matches!(self.current_phase, EpochPhase::Collect | EpochPhase::Seal)
    }

    /// Guard a withdrawal attempt. Returns `Ok(())` if allowed,
    /// or [`OpenmatchError::WithdrawLockedDuringSettle`] if blocked.
    pub fn check_withdraw(&self) -> Result<()> {
        if self.withdrawals_allowed() {
            Ok(())
        } else {
            Err(OpenmatchError::WithdrawLockedDuringSettle)
        }
    }
}

impl Default for WithdrawLock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_phase_allows_withdraw() {
        let lock = WithdrawLock::new();
        assert!(lock.withdrawals_allowed());
        assert!(lock.check_withdraw().is_ok());
    }

    #[test]
    fn seal_phase_allows_withdraw() {
        let mut lock = WithdrawLock::new();
        lock.set_phase(EpochPhase::Seal);
        assert!(lock.withdrawals_allowed());
        assert!(lock.check_withdraw().is_ok());
    }

    #[test]
    fn match_phase_blocks_withdraw() {
        let mut lock = WithdrawLock::new();
        lock.set_phase(EpochPhase::Match);
        assert!(!lock.withdrawals_allowed());
        assert!(lock.check_withdraw().is_err());
    }

    #[test]
    fn finalize_phase_blocks_withdraw() {
        let mut lock = WithdrawLock::new();
        lock.set_phase(EpochPhase::Finalize);
        assert!(!lock.withdrawals_allowed());
        let err = lock.check_withdraw().unwrap_err();
        assert!(matches!(err, OpenmatchError::WithdrawLockedDuringSettle));
    }

    #[test]
    fn phase_transitions_update_lock() {
        let mut lock = WithdrawLock::new();
        assert!(lock.withdrawals_allowed());

        lock.set_phase(EpochPhase::Seal);
        assert!(lock.withdrawals_allowed());

        lock.set_phase(EpochPhase::Match);
        assert!(!lock.withdrawals_allowed());

        lock.set_phase(EpochPhase::Finalize);
        assert!(!lock.withdrawals_allowed());

        lock.set_phase(EpochPhase::Collect);
        assert!(lock.withdrawals_allowed());
    }
}
