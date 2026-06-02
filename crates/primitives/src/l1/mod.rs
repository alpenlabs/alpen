// Re-export from strata-btc-types and strata-identifiers for backward compatibility
pub use strata_btc_types::{payload, *};
pub use strata_identifiers::{L1BlockCommitment, L1BlockId, L1Height};

/// Number of confirmations an L1 block has under `current_tip`, counting the
/// block itself as one confirmation.
///
/// A block at the tip has 1 confirmation; one block below tip has 2; etc.
/// Observation heights above the tip saturate to 0.
pub fn l1_confirmations(observed_height: L1Height, current_tip: L1Height) -> u32 {
    if observed_height > current_tip {
        return 0;
    }
    current_tip
        .saturating_sub(observed_height)
        .saturating_add(1)
}

/// A single computation logic for whether an L1 block at `observed_height` is buried deep enough
/// under `current_tip` to be considered reorg-safe.
pub fn is_l1_reorg_safe(
    observed_height: L1Height,
    current_tip: L1Height,
    l1_reorg_safe_depth: u32,
) -> bool {
    l1_confirmations(observed_height, current_tip) >= l1_reorg_safe_depth.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirmations_at_tip_is_one() {
        assert_eq!(l1_confirmations(100, 100), 1);
    }

    #[test]
    fn confirmations_grows_with_burial() {
        assert_eq!(l1_confirmations(98, 100), 3);
    }

    #[test]
    fn confirmations_above_tip_is_zero() {
        assert_eq!(l1_confirmations(101, 100), 0);
    }

    #[test]
    fn reorg_safe_exactly_at_threshold() {
        // depth=3 means: need >= 3 confirmations. tip=102, obs=100 => 3 confs.
        assert!(is_l1_reorg_safe(100, 102, 3));
    }

    #[test]
    fn reorg_safe_one_below_threshold() {
        // tip=101, obs=100 => 2 confs, depth=3 not satisfied.
        assert!(!is_l1_reorg_safe(100, 101, 3));
    }

    #[test]
    fn reorg_safe_depth_zero_clamped_to_one() {
        // depth=0 must not mark the tip block trivially safe.
        // tip=100, obs=100 => 1 conf, clamped depth=1 satisfied.
        assert!(is_l1_reorg_safe(100, 100, 0));
        // But obs above tip never qualifies.
        assert!(!is_l1_reorg_safe(101, 100, 0));
    }
}
