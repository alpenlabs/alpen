//! Epoch sealing policy for OL block assembly.
//!
//! The sealing policy determines when an epoch should be sealed, i.e. when to
//! create a terminal block. This is a batch production concern, not an STF
//! concern.

use std::cmp::Ordering;
use std::fmt::Debug;

use strata_identifiers::Slot;
use strata_ol_chain_types_v1::MAX_SEALING_MANIFEST_COUNT;

use crate::checkpoint_size::{
    CheckpointLimit, CheckpointSizeVerdict, LogMetrics, checkpoint_size_verdict,
};

/// Resource stats used by the epoch sealing policy.
///
/// All values are epoch-cumulative for the candidate state being checked.
/// Block assembly builds this snapshot incrementally before admitting a
/// candidate resource, such as a transaction or manifest sequence.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EpochSealingResourceStats {
    da_diff_size: usize,
    log: LogMetrics,
    manifest_count: u32,
}

impl EpochSealingResourceStats {
    /// Creates a new resource stats snapshot.
    pub(crate) fn new(da_diff_size: usize, log: LogMetrics, manifest_count: u32) -> Self {
        Self {
            da_diff_size,
            log,
            manifest_count,
        }
    }

    /// Returns the estimated DA diff size.
    pub(crate) fn da_diff_size(&self) -> usize {
        self.da_diff_size
    }

    /// Returns the OL log metrics.
    pub(crate) fn log(&self) -> &LogMetrics {
        &self.log
    }

    /// Returns the ASM manifest count.
    pub(crate) fn manifest_count(&self) -> u32 {
        self.manifest_count
    }
}

/// A candidate admission action requested by a sealing limit.
///
/// Variants are ordered so `max()` yields the most restrictive action.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum EpochSealingLimitAction {
    /// The candidate remains below the sealing threshold.
    #[default]
    Continue,

    /// Admit the candidate and seal this block.
    SealAfterAdmit,

    /// Exclude the candidate; assembly decides whether its deferral requests a seal.
    RejectCandidate,
}

impl EpochSealingLimitAction {
    fn should_seal(self) -> bool {
        self != Self::Continue
    }
}

/// Verdict from checking candidate values against sealing limits.
///
/// The verdict preserves individual checkpoint-size and manifest-count actions
/// so multiple crossed limits can be observed together.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EpochSealingLimitVerdict {
    da_diff: EpochSealingLimitAction,
    log_count: EpochSealingLimitAction,
    log_payload_bytes: EpochSealingLimitAction,
    envelope: EpochSealingLimitAction,
    manifest_count: EpochSealingLimitAction,
}

impl EpochSealingLimitVerdict {
    /// Creates a verdict with no limits reached.
    pub(crate) fn within_limits() -> Self {
        Self::default()
    }

    /// Records a checkpoint limit, keeping the stricter action.
    pub(crate) fn record_checkpoint_limit(
        &mut self,
        limit: CheckpointLimit,
        action: EpochSealingLimitAction,
    ) {
        let existing = match limit {
            CheckpointLimit::DaDiff => &mut self.da_diff,
            CheckpointLimit::LogCount => &mut self.log_count,
            CheckpointLimit::LogPayloadBytes => &mut self.log_payload_bytes,
            CheckpointLimit::Envelope => &mut self.envelope,
        };
        *existing = (*existing).max(action);
    }

    /// Merges another verdict into this one, keeping the stricter action for each limit.
    pub(crate) fn merge(&mut self, other: Self) {
        self.da_diff = self.da_diff.max(other.da_diff);
        self.log_count = self.log_count.max(other.log_count);
        self.log_payload_bytes = self.log_payload_bytes.max(other.log_payload_bytes);
        self.envelope = self.envelope.max(other.envelope);
        self.manifest_count = self.manifest_count.max(other.manifest_count);
    }

    fn should_seal(&self) -> bool {
        self.most_restrictive_action().should_seal()
    }

    /// Returns the strictest action across checkpoint DA, log, and envelope limits.
    pub(crate) fn checkpoint_size_action(&self) -> EpochSealingLimitAction {
        self.da_diff
            .max(self.log_count)
            .max(self.log_payload_bytes)
            .max(self.envelope)
    }

    /// Returns whether checkpoint log count or payload bytes reached a hard limit.
    pub(crate) fn checkpoint_logs_exceeded(&self) -> bool {
        self.log_count == EpochSealingLimitAction::RejectCandidate
            || self.log_payload_bytes == EpochSealingLimitAction::RejectCandidate
    }

    /// Returns the manifest-count limit action.
    #[cfg(test)]
    pub(crate) fn manifest_count_action(&self) -> EpochSealingLimitAction {
        self.manifest_count
    }

    pub(crate) fn most_restrictive_action(&self) -> EpochSealingLimitAction {
        self.checkpoint_size_action().max(self.manifest_count)
    }

    fn seal_trigger(&self) -> Option<EpochSealTrigger> {
        self.should_seal()
            .then(|| EpochSealTrigger::Limits(self.clone()))
    }
}

/// Trigger that requested an epoch seal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EpochSealTrigger {
    /// The configured sealing cadence requested a terminal block.
    Cadence,
    /// One or more non-cadence limits requested a terminal block.
    Limits(EpochSealingLimitVerdict),
}

/// Decision returned by an [`EpochSealingPolicy`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EpochSealingDecision {
    /// Seal the epoch for the given trigger.
    Seal(EpochSealTrigger),

    /// Keep building non-terminal blocks.
    Continue,
}

impl EpochSealingDecision {
    /// Returns `true` when this decision seals the epoch.
    pub(crate) fn should_seal(&self) -> bool {
        matches!(self, Self::Seal(_))
    }
}

/// Trait for fixed epoch sealing cadence.
pub trait CadencePolicy: Send + Sync + Debug + 'static {
    /// Returns `true` if this cadence seals at `slot`.
    fn seals_at_slot(&self, slot: Slot) -> bool;
}

/// Trait for deciding when to seal an epoch.
pub trait EpochSealingPolicy: Send + Sync + Debug + 'static {
    /// Checks candidate resource stats against sealing limits.
    fn check_limits(&self, stats: &EpochSealingResourceStats) -> EpochSealingLimitVerdict;

    /// Decides whether a terminal block should be created.
    fn should_seal_epoch(
        &self,
        slot: Slot,
        limit_verdict: &EpochSealingLimitVerdict,
    ) -> EpochSealingDecision;
}

/// Sealing policy that checks checkpoint size, manifest count, and cadence.
#[derive(Debug)]
pub struct LimitAwareSealing<C: CadencePolicy> {
    cadence: C,
}

impl<C: CadencePolicy> LimitAwareSealing<C> {
    /// Creates a sealing policy with the given cadence.
    pub fn new(cadence: C) -> Self {
        Self { cadence }
    }
}

impl<C: CadencePolicy> EpochSealingPolicy for LimitAwareSealing<C> {
    fn check_limits(&self, stats: &EpochSealingResourceStats) -> EpochSealingLimitVerdict {
        let checkpoint_action =
            |limit| match checkpoint_size_verdict(limit, stats.da_diff_size(), stats.log()) {
                CheckpointSizeVerdict::WithinLimits => EpochSealingLimitAction::Continue,
                CheckpointSizeVerdict::SoftLimitReached => EpochSealingLimitAction::SealAfterAdmit,
                CheckpointSizeVerdict::HardLimitExceeded => {
                    EpochSealingLimitAction::RejectCandidate
                }
            };

        // The epoch manifest budget currently uses the same numeric value as
        // the per-block SSZ container bound.
        let max_epoch_manifests = MAX_SEALING_MANIFEST_COUNT as u32;
        let manifest_count_action = match stats.manifest_count().cmp(&max_epoch_manifests) {
            Ordering::Less => EpochSealingLimitAction::Continue,
            Ordering::Equal => EpochSealingLimitAction::SealAfterAdmit,
            Ordering::Greater => EpochSealingLimitAction::RejectCandidate,
        };

        EpochSealingLimitVerdict {
            da_diff: checkpoint_action(CheckpointLimit::DaDiff),
            log_count: checkpoint_action(CheckpointLimit::LogCount),
            log_payload_bytes: checkpoint_action(CheckpointLimit::LogPayloadBytes),
            envelope: checkpoint_action(CheckpointLimit::Envelope),
            manifest_count: manifest_count_action,
        }
    }

    fn should_seal_epoch(
        &self,
        slot: Slot,
        limit_verdict: &EpochSealingLimitVerdict,
    ) -> EpochSealingDecision {
        if let Some(trigger) = limit_verdict.seal_trigger() {
            EpochSealingDecision::Seal(trigger)
        } else if self.cadence.seals_at_slot(slot) {
            EpochSealingDecision::Seal(EpochSealTrigger::Cadence)
        } else {
            EpochSealingDecision::Continue
        }
    }
}

/// Fixed slot-count sealing cadence.
///
/// Seals an epoch at slots that are multiples of `slots_per_epoch`.
/// This includes genesis (slot 0) since `0.is_multiple_of(n)` is true.
#[derive(Debug, Clone)]
pub struct FixedSlotSealing {
    slots_per_epoch: u64,
}

impl FixedSlotSealing {
    /// Creates a new fixed slot sealing cadence.
    ///
    /// # Panics
    ///
    /// Panics if `slots_per_epoch` is 0.
    pub fn new(slots_per_epoch: u64) -> Self {
        assert!(slots_per_epoch > 0, "slots_per_epoch must be > 0");
        Self { slots_per_epoch }
    }
}

impl CadencePolicy for FixedSlotSealing {
    fn seals_at_slot(&self, slot: Slot) -> bool {
        // Terminal slots are multiples of slots_per_epoch: 0, N, 2N, 3N, ...
        // Genesis (slot 0) is terminal since 0.is_multiple_of(n) == true.
        slot.is_multiple_of(self.slots_per_epoch)
    }
}

#[cfg(test)]
mod fixed_slot_sealing_tests {
    use strata_asm_checkpoint_types::{MAX_OL_LOGS_PER_CHECKPOINT, OL_DA_DIFF_MAX_SIZE};
    use strata_ol_tx_policy::MAX_TOTAL_LOG_PAYLOAD_BYTES;

    use super::*;

    #[test]
    fn test_genesis_is_terminal() {
        let sealing = FixedSlotSealing::new(10);
        assert!(sealing.seals_at_slot(0));
    }

    #[test]
    fn test_intermediate_slots_not_terminal() {
        let sealing = FixedSlotSealing::new(10);
        for slot in 1..10 {
            assert!(
                !sealing.seals_at_slot(slot),
                "slot {slot} should not be terminal"
            );
        }
    }

    #[test]
    fn test_epoch_boundaries() {
        let sealing = FixedSlotSealing::new(10);
        assert!(sealing.seals_at_slot(0));
        assert!(sealing.seals_at_slot(10));
        assert!(sealing.seals_at_slot(20));
        assert!(sealing.seals_at_slot(30));

        assert!(!sealing.seals_at_slot(9));
        assert!(!sealing.seals_at_slot(11));
        assert!(!sealing.seals_at_slot(19));
        assert!(!sealing.seals_at_slot(21));
    }

    #[test]
    fn test_checkpoint_size_limit_seals_through_policy_decision() {
        let sealing = LimitAwareSealing::new(FixedSlotSealing::new(10));
        let stats = EpochSealingResourceStats::new(
            0,
            LogMetrics {
                count: MAX_OL_LOGS_PER_CHECKPOINT as usize + 1,
                ..Default::default()
            },
            0,
        );
        let verdict = sealing.check_limits(&stats);
        let decision = sealing.should_seal_epoch(1, &verdict);

        assert_eq!(
            verdict.checkpoint_size_action(),
            EpochSealingLimitAction::RejectCandidate,
            "checkpoint hard limit should reject the current candidate"
        );
        assert_eq!(
            decision,
            EpochSealingDecision::Seal(EpochSealTrigger::Limits(verdict))
        );
    }

    #[test]
    fn test_multiple_limits_are_preserved_in_policy_decision() {
        let sealing = LimitAwareSealing::new(FixedSlotSealing::new(10));
        let stats = EpochSealingResourceStats::new(
            0,
            LogMetrics {
                count: MAX_OL_LOGS_PER_CHECKPOINT as usize + 1,
                ..Default::default()
            },
            MAX_SEALING_MANIFEST_COUNT as u32,
        );
        let verdict = sealing.check_limits(&stats);

        assert_eq!(
            verdict.checkpoint_size_action(),
            EpochSealingLimitAction::RejectCandidate
        );
        assert_eq!(
            verdict.manifest_count_action(),
            EpochSealingLimitAction::SealAfterAdmit
        );

        let decision = sealing.should_seal_epoch(1, &verdict);
        assert_eq!(
            decision,
            EpochSealingDecision::Seal(EpochSealTrigger::Limits(verdict))
        );
    }

    #[test]
    fn test_checkpoint_resources_preserve_hard_and_soft_actions() {
        let sealing = LimitAwareSealing::new(FixedSlotSealing::new(10));
        let stats = EpochSealingResourceStats::new(
            OL_DA_DIFF_MAX_SIZE as usize,
            LogMetrics {
                count: MAX_OL_LOGS_PER_CHECKPOINT as usize * 9 / 10,
                total_payload: MAX_TOTAL_LOG_PAYLOAD_BYTES + 1,
                ssz_size: 0,
            },
            0,
        );
        let verdict = sealing.check_limits(&stats);

        assert_eq!(
            verdict,
            EpochSealingLimitVerdict {
                da_diff: EpochSealingLimitAction::RejectCandidate,
                log_count: EpochSealingLimitAction::SealAfterAdmit,
                log_payload_bytes: EpochSealingLimitAction::RejectCandidate,
                ..Default::default()
            }
        );
        assert!(verdict.checkpoint_logs_exceeded());
        assert_eq!(
            sealing.should_seal_epoch(1, &verdict),
            EpochSealingDecision::Seal(EpochSealTrigger::Limits(verdict))
        );
    }

    #[test]
    fn test_da_failure_does_not_count_as_hard_log_failure() {
        let sealing = LimitAwareSealing::new(FixedSlotSealing::new(10));
        let stats =
            EpochSealingResourceStats::new(OL_DA_DIFF_MAX_SIZE as usize, LogMetrics::default(), 0);
        let verdict = sealing.check_limits(&stats);

        assert_eq!(verdict.da_diff, EpochSealingLimitAction::RejectCandidate);
        assert!(!verdict.checkpoint_logs_exceeded());
    }

    #[test]
    fn test_envelope_failure_does_not_count_as_hard_log_failure() {
        let sealing = LimitAwareSealing::new(FixedSlotSealing::new(10));
        let stats = EpochSealingResourceStats::new(
            200_000,
            LogMetrics {
                count: 16_000,
                total_payload: 0,
                ssz_size: 16_000 * 12,
            },
            0,
        );
        let verdict = sealing.check_limits(&stats);

        assert_eq!(verdict.envelope, EpochSealingLimitAction::RejectCandidate);
        assert!(!verdict.checkpoint_logs_exceeded());
    }

    #[test]
    fn test_manifest_count_limit_actions() {
        let sealing = LimitAwareSealing::new(FixedSlotSealing::new(10));
        let max_manifests = MAX_SEALING_MANIFEST_COUNT as u32;

        for (manifest_count, expected_action) in [
            (max_manifests - 1, EpochSealingLimitAction::Continue),
            (max_manifests, EpochSealingLimitAction::SealAfterAdmit),
            (max_manifests + 1, EpochSealingLimitAction::RejectCandidate),
        ] {
            let stats = EpochSealingResourceStats::new(0, LogMetrics::default(), manifest_count);
            let verdict = sealing.check_limits(&stats);

            assert_eq!(
                verdict.manifest_count_action(),
                expected_action,
                "manifest count: {manifest_count}"
            );
        }
    }

    #[test]
    fn test_merge_keeps_stricter_checkpoint_action() {
        let mut verdict = EpochSealingLimitVerdict {
            log_count: EpochSealingLimitAction::RejectCandidate,
            ..Default::default()
        };
        let weaker = EpochSealingLimitVerdict {
            log_count: EpochSealingLimitAction::SealAfterAdmit,
            ..Default::default()
        };

        verdict.merge(weaker);

        assert_eq!(
            verdict.checkpoint_size_action(),
            EpochSealingLimitAction::RejectCandidate
        );
    }

    #[test]
    fn test_merge_preserves_distinct_limits() {
        let mut tx_verdict = EpochSealingLimitVerdict {
            log_count: EpochSealingLimitAction::SealAfterAdmit,
            ..Default::default()
        };
        let manifest_verdict = EpochSealingLimitVerdict {
            manifest_count: EpochSealingLimitAction::RejectCandidate,
            ..Default::default()
        };

        tx_verdict.merge(manifest_verdict);

        assert_eq!(
            tx_verdict.checkpoint_size_action(),
            EpochSealingLimitAction::SealAfterAdmit
        );
        assert_eq!(
            tx_verdict.manifest_count_action(),
            EpochSealingLimitAction::RejectCandidate
        );
        assert!(tx_verdict.should_seal());
    }

    #[test]
    fn test_merge_keeps_stricter_manifest_action() {
        let mut verdict = EpochSealingLimitVerdict {
            manifest_count: EpochSealingLimitAction::SealAfterAdmit,
            ..Default::default()
        };
        let stricter = EpochSealingLimitVerdict {
            manifest_count: EpochSealingLimitAction::RejectCandidate,
            ..Default::default()
        };

        verdict.merge(stricter);

        assert_eq!(
            verdict.manifest_count_action(),
            EpochSealingLimitAction::RejectCandidate
        );
    }

    #[test]
    fn test_cadence_seals_through_policy_decision() {
        let sealing = LimitAwareSealing::new(FixedSlotSealing::new(10));
        let verdict = EpochSealingLimitVerdict::within_limits();
        let decision = sealing.should_seal_epoch(10, &verdict);

        assert_eq!(
            decision,
            EpochSealingDecision::Seal(EpochSealTrigger::Cadence)
        );
    }

    #[test]
    fn test_policy_decision_non_terminal() {
        let sealing = LimitAwareSealing::new(FixedSlotSealing::new(10));
        let verdict = EpochSealingLimitVerdict::within_limits();
        let decision = sealing.should_seal_epoch(1, &verdict);

        assert_eq!(decision, EpochSealingDecision::Continue);
    }

    #[test]
    #[should_panic(expected = "slots_per_epoch must be > 0")]
    fn test_zero_slots_per_epoch_panics() {
        let _ = FixedSlotSealing::new(0);
    }
}
