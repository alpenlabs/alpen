//! Epoch sealing policy for OL block assembly.
//!
//! The sealing policy determines when an epoch should be sealed (i.e., when to create
//! a terminal block). This is a batch production concern, not an STF concern.

use std::fmt::Debug;

use strata_identifiers::Slot;

/// Trait for deciding when to seal an epoch.
///
/// Implementations define the threshold logic for determining when an epoch
/// should be sealed (e.g., by slot count, DA size, or a combination).
pub trait EpochSealingPolicy: Send + Sync + Debug {
    /// Returns `true` if a terminal block should be created at this slot.
    fn should_seal_epoch(&self, slot: Slot) -> bool;
}

/// Fixed slot-count sealing policy.
///
/// Seals an epoch at slots that are multiples of `slots_per_epoch`.
/// This includes genesis (slot 0) since `0.is_multiple_of(n)` is true.
#[derive(Debug, Clone)]
pub struct FixedSlotSealing {
    slots_per_epoch: u64,
}

impl FixedSlotSealing {
    /// Creates a new fixed slot sealing policy.
    ///
    /// # Panics
    ///
    /// Panics if `slots_per_epoch` is 0.
    pub fn new(slots_per_epoch: u64) -> Self {
        assert!(slots_per_epoch > 0, "slots_per_epoch must be > 0");
        Self { slots_per_epoch }
    }

    /// Returns the configured slots per epoch.
    pub fn slots_per_epoch(&self) -> u64 {
        self.slots_per_epoch
    }
}

impl EpochSealingPolicy for FixedSlotSealing {
    fn should_seal_epoch(&self, slot: Slot) -> bool {
        // Terminal slots are multiples of slots_per_epoch: 0, N, 2N, 3N, ...
        // Genesis (slot 0) is terminal since 0.is_multiple_of(n) == true
        slot.is_multiple_of(self.slots_per_epoch)
    }
}

#[cfg(test)]
mod fixed_slot_sealing_tests {
    use super::*;

    #[test]
    fn test_genesis_is_terminal() {
        let sealing = FixedSlotSealing::new(10);
        assert!(sealing.should_seal_epoch(0));
    }

    #[test]
    fn test_intermediate_slots_not_terminal() {
        let sealing = FixedSlotSealing::new(10);
        for slot in 1..10 {
            assert!(
                !sealing.should_seal_epoch(slot),
                "slot {slot} should not be terminal"
            );
        }
    }

    #[test]
    fn test_epoch_boundaries() {
        let sealing = FixedSlotSealing::new(10);
        // Terminal slots: 0, 10, 20, 30, ...
        assert!(sealing.should_seal_epoch(0));
        assert!(sealing.should_seal_epoch(10));
        assert!(sealing.should_seal_epoch(20));
        assert!(sealing.should_seal_epoch(30));

        // Non-terminal around boundaries
        assert!(!sealing.should_seal_epoch(9));
        assert!(!sealing.should_seal_epoch(11));
        assert!(!sealing.should_seal_epoch(19));
        assert!(!sealing.should_seal_epoch(21));
    }

    #[test]
    #[should_panic(expected = "slots_per_epoch must be > 0")]
    fn test_zero_slots_per_epoch_panics() {
        let _ = FixedSlotSealing::new(0);
    }
}
