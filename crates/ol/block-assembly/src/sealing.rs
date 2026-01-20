//! Epoch sealing policy for OL block assembly.
//!
//! The sealing policy determines when an epoch should be sealed (i.e., when to create
//! a terminal block). This is a batch production concern, not an STF concern—the STF
//! executes whatever block it receives, while block assembly decides whether to create
//! a terminal block by consulting the sealing policy.
//!
//! # Flexibility
//!
//! The [`EpochSealingPolicy`] trait is designed to support various policies:
//! - **Slot-based** (v1): Seal every N slots using [`FixedSlotSealing`]
//! - **Diff-based** (future): Seal when DA size approaches limit
//!
//! For complex policies that need storage access, implementations can hold
//! storage references and compute state diffs on-the-fly during `should_seal()`.

use std::fmt::Debug;

use strata_identifiers::Slot;

/// Trait for deciding when to seal an epoch.
///
/// Implementations define the threshold logic for determining when an epoch
/// should be sealed (e.g., by slot count, DA size, or a combination).
///
/// # Design
///
/// For simple policies (slot-based), only the slot number is needed.
/// For complex policies (diff-based), implementations can hold storage
/// references and compute diffs on-the-fly when `should_seal()` is called.
pub trait EpochSealingPolicy: Send + Sync + Debug {
    /// Check if epoch should be sealed at the given slot.
    ///
    /// Returns `true` if a terminal block should be created at this slot,
    /// `false` for a common block.
    fn should_seal(&self, slot: Slot) -> bool;
}

/// Fixed slot-count sealing policy.
///
/// Seals an epoch every N slots. This is the simplest sealing policy:
/// - Slot 0 (genesis): Not terminal
/// - Slot 1: Terminal (seals epoch 0)
/// - Subsequent slots: Terminal at slot 1, 1+N, 1+2N, ...
#[derive(Debug, Clone)]
pub struct FixedSlotSealing {
    slots_per_epoch: u64,
}

impl FixedSlotSealing {
    /// Create a new fixed slot sealing policy.
    ///
    /// # Arguments
    ///
    /// * `slots_per_epoch` - Number of slots per epoch (must be > 0)
    ///
    /// # Panics
    ///
    /// Panics if `slots_per_epoch` is 0.
    pub fn new(slots_per_epoch: u64) -> Self {
        assert!(slots_per_epoch > 0, "slots_per_epoch must be > 0");
        Self { slots_per_epoch }
    }

    /// Get the configured slots per epoch.
    pub fn slots_per_epoch(&self) -> u64 {
        self.slots_per_epoch
    }
}

impl EpochSealingPolicy for FixedSlotSealing {
    fn should_seal(&self, slot: Slot) -> bool {
        if slot == 0 {
            // Genesis block is not terminal
            false
        } else if slot == 1 {
            // First real block seals epoch 0
            true
        } else {
            // Terminal at slot 1, 1+N, 1+2N, ...
            // i.e., (slot - 1) is a multiple of slots_per_epoch
            (slot - 1).is_multiple_of(self.slots_per_epoch)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_genesis_not_terminal() {
        let sealing = FixedSlotSealing::new(64);
        assert!(!sealing.should_seal(0));
    }

    #[test]
    fn test_slot_1_terminal() {
        let sealing = FixedSlotSealing::new(64);
        assert!(sealing.should_seal(1));
    }

    #[test]
    fn test_intermediate_slots_not_terminal() {
        let sealing = FixedSlotSealing::new(64);

        // Slots 2-64 should not be terminal
        for slot in 2..=64 {
            assert!(
                !sealing.should_seal(slot),
                "slot {slot} should not be terminal"
            );
        }
    }

    #[test]
    fn test_epoch_boundaries() {
        let sealing = FixedSlotSealing::new(64);

        // Terminal slots: 1, 65, 129, 193, ...
        assert!(sealing.should_seal(1));
        assert!(sealing.should_seal(65));
        assert!(sealing.should_seal(129));
        assert!(sealing.should_seal(193));

        // Non-terminal around boundaries
        assert!(!sealing.should_seal(64));
        assert!(!sealing.should_seal(66));
        assert!(!sealing.should_seal(128));
        assert!(!sealing.should_seal(130));
    }

    #[test]
    fn test_small_epoch() {
        let sealing = FixedSlotSealing::new(3);

        // Terminal slots: 1, 4, 7, 10, ...
        assert!(!sealing.should_seal(0));
        assert!(sealing.should_seal(1));
        assert!(!sealing.should_seal(2));
        assert!(!sealing.should_seal(3));
        assert!(sealing.should_seal(4));
        assert!(!sealing.should_seal(5));
        assert!(!sealing.should_seal(6));
        assert!(sealing.should_seal(7));
    }

    #[test]
    fn test_slots_per_epoch_getter() {
        let sealing = FixedSlotSealing::new(100);
        assert_eq!(sealing.slots_per_epoch(), 100);
    }

    #[test]
    #[should_panic(expected = "slots_per_epoch must be > 0")]
    fn test_zero_slots_per_epoch_panics() {
        let _ = FixedSlotSealing::new(0);
    }
}
