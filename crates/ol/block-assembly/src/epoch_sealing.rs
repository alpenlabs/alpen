//! Epoch sealing policy for OL block assembly.
//!
//! The sealing policy determines when an epoch should be sealed, i.e. when to
//! create a terminal block. This is a batch production concern, not an STF
//! concern.

use std::{cmp::Ordering, fmt::Debug};

use strata_identifiers::Slot;
use strata_ol_chain_types_new::MAX_SEALING_MANIFEST_COUNT;

use crate::checkpoint_size::{CheckpointSizeVerdict, LogMetrics, checkpoint_size_verdict};

/// Resource stats used by sealing-limit rules.
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

    /// Reject the candidate and seal with the state before it.
    RejectCandidate,
}

impl EpochSealingLimitAction {
    fn should_seal(self) -> bool {
        self != Self::Continue
    }
}

/// Epoch sealing limit identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EpochSealingLimit {
    /// Estimated checkpoint payload or sidecar size.
    CheckpointSize,

    /// Epoch-cumulative ASM manifest count.
    ManifestCount,
}

/// Verdict from checking candidate values against sealing limits.
///
/// The verdict preserves checkpoint-size and manifest-count actions separately
/// so multiple crossed limits can be observed together.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EpochSealingLimitVerdict {
    actions: Vec<(EpochSealingLimit, EpochSealingLimitAction)>,
}

impl EpochSealingLimitVerdict {
    /// Creates a verdict with no limits reached.
    pub(crate) fn within_limits() -> Self {
        Self::default()
    }

    fn from_actions(
        actions: impl IntoIterator<Item = (EpochSealingLimit, EpochSealingLimitAction)>,
    ) -> Self {
        let mut verdict = Self::within_limits();
        for (limit, action) in actions {
            verdict.record(limit, action);
        }
        verdict
    }

    fn record(&mut self, limit: EpochSealingLimit, action: EpochSealingLimitAction) {
        if action == EpochSealingLimitAction::Continue {
            return;
        }

        if let Some((_, existing)) = self
            .actions
            .iter_mut()
            .find(|(existing_limit, _)| *existing_limit == limit)
        {
            *existing = (*existing).max(action);
        } else {
            self.actions.push((limit, action));
        }
    }

    /// Merges another verdict into this one, keeping the stricter action for each limit.
    pub(crate) fn merge(&mut self, other: Self) {
        for (limit, action) in other.actions {
            self.record(limit, action);
        }
    }

    fn should_seal(&self) -> bool {
        self.most_restrictive_action().should_seal()
    }

    /// Returns the checkpoint-size limit action.
    pub(crate) fn checkpoint_size_action(&self) -> EpochSealingLimitAction {
        self.action_for(EpochSealingLimit::CheckpointSize)
    }

    /// Returns the manifest-count limit action.
    #[cfg(test)]
    pub(crate) fn manifest_count_action(&self) -> EpochSealingLimitAction {
        self.action_for(EpochSealingLimit::ManifestCount)
    }

    fn action_for(&self, limit: EpochSealingLimit) -> EpochSealingLimitAction {
        self.actions
            .iter()
            .find_map(|(id, action)| (*id == limit).then_some(*action))
            .unwrap_or_default()
    }

    fn actions(&self) -> impl Iterator<Item = (EpochSealingLimit, EpochSealingLimitAction)> + '_ {
        self.actions.iter().copied()
    }

    pub(crate) fn most_restrictive_action(&self) -> EpochSealingLimitAction {
        self.actions()
            .map(|(_, action)| action)
            .max()
            .unwrap_or_default()
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

/// Trait for a concrete sealing-limit rule.
pub(crate) trait SealingLimitRule: Send + Sync + Debug + 'static {
    /// Returns the limit checked by this rule.
    fn limit(&self) -> EpochSealingLimit;

    /// Checks the resource stats against this rule.
    fn check(&self, stats: &EpochSealingResourceStats) -> EpochSealingLimitAction;
}

/// Checkpoint-size sealing limit rule.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct CheckpointSizeRule;

impl SealingLimitRule for CheckpointSizeRule {
    fn limit(&self) -> EpochSealingLimit {
        EpochSealingLimit::CheckpointSize
    }

    fn check(&self, stats: &EpochSealingResourceStats) -> EpochSealingLimitAction {
        match checkpoint_size_verdict(stats.da_diff_size(), stats.log()) {
            CheckpointSizeVerdict::WithinLimits => EpochSealingLimitAction::Continue,
            CheckpointSizeVerdict::SoftLimitReached => EpochSealingLimitAction::SealAfterAdmit,
            CheckpointSizeVerdict::HardLimitExceeded => EpochSealingLimitAction::RejectCandidate,
        }
    }
}

/// Epoch-cumulative ASM manifest-count sealing limit rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ManifestCountRule {
    max_epoch_manifests: u32,
}

impl ManifestCountRule {
    /// Creates a new manifest-count rule.
    ///
    /// `max_epoch_manifests` is a sealing budget, not the per-block SSZ
    /// container bound. The default currently uses the same numeric value until
    /// a separate configured budget is introduced.
    fn new(max_epoch_manifests: u32) -> Self {
        Self {
            max_epoch_manifests,
        }
    }
}

impl Default for ManifestCountRule {
    fn default() -> Self {
        Self::new(MAX_SEALING_MANIFEST_COUNT as u32)
    }
}

impl SealingLimitRule for ManifestCountRule {
    fn limit(&self) -> EpochSealingLimit {
        EpochSealingLimit::ManifestCount
    }

    fn check(&self, stats: &EpochSealingResourceStats) -> EpochSealingLimitAction {
        match stats.manifest_count().cmp(&self.max_epoch_manifests) {
            Ordering::Less => EpochSealingLimitAction::Continue,
            Ordering::Equal => EpochSealingLimitAction::SealAfterAdmit,
            Ordering::Greater => EpochSealingLimitAction::RejectCandidate,
        }
    }
}

/// Construction-time context for sealing limit checks.
#[derive(Debug)]
struct EpochSealingPolicyContext {
    limits: Vec<Box<dyn SealingLimitRule>>,
}

impl Default for EpochSealingPolicyContext {
    fn default() -> Self {
        Self {
            limits: vec![
                Box::new(CheckpointSizeRule),
                Box::new(ManifestCountRule::default()),
            ],
        }
    }
}

impl EpochSealingPolicyContext {
    fn check_limits(&self, stats: &EpochSealingResourceStats) -> EpochSealingLimitVerdict {
        EpochSealingLimitVerdict::from_actions(
            self.limits
                .iter()
                .map(|rule| (rule.limit(), rule.check(stats))),
        )
    }
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

/// Sealing policy that combines a cadence policy with sealing-limit rules.
#[derive(Debug)]
pub struct LimitAwareSealing<C: CadencePolicy> {
    cadence: C,
    context: EpochSealingPolicyContext,
}

impl<C: CadencePolicy> LimitAwareSealing<C> {
    /// Creates a limit-aware policy with default sealing-limit rules.
    pub fn new(cadence: C) -> Self {
        Self::with_context(cadence, EpochSealingPolicyContext::default())
    }

    fn with_context(cadence: C, context: EpochSealingPolicyContext) -> Self {
        Self { cadence, context }
    }
}

impl<C: CadencePolicy> EpochSealingPolicy for LimitAwareSealing<C> {
    fn check_limits(&self, stats: &EpochSealingResourceStats) -> EpochSealingLimitVerdict {
        self.context.check_limits(stats)
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
    use strata_asm_proto_checkpoint_types::MAX_OL_LOGS_PER_CHECKPOINT;

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
                count: MAX_OL_LOGS_PER_CHECKPOINT as usize,
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
                count: MAX_OL_LOGS_PER_CHECKPOINT as usize,
                ..Default::default()
            },
            MAX_SEALING_MANIFEST_COUNT as u32,
        );
        let verdict = sealing.check_limits(&stats);

        assert_eq!(
            verdict.checkpoint_size_action(),
            EpochSealingLimitAction::RejectCandidate
        );
        assert!(verdict.actions().any(|(limit, action)| {
            limit == EpochSealingLimit::ManifestCount
                && action == EpochSealingLimitAction::SealAfterAdmit
        }));

        let decision = sealing.should_seal_epoch(1, &verdict);
        assert_eq!(
            decision,
            EpochSealingDecision::Seal(EpochSealTrigger::Limits(verdict))
        );
    }

    #[test]
    fn test_manifest_count_rule_actions() {
        let rule = ManifestCountRule::new(3);

        assert_eq!(
            rule.check(&EpochSealingResourceStats::new(0, LogMetrics::default(), 2)),
            EpochSealingLimitAction::Continue
        );
        assert_eq!(
            rule.check(&EpochSealingResourceStats::new(0, LogMetrics::default(), 3)),
            EpochSealingLimitAction::SealAfterAdmit
        );
        assert_eq!(
            rule.check(&EpochSealingResourceStats::new(0, LogMetrics::default(), 4)),
            EpochSealingLimitAction::RejectCandidate
        );
    }

    #[test]
    fn test_duplicate_limit_action_keeps_most_restrictive() {
        let verdict = EpochSealingLimitVerdict::from_actions([
            (
                EpochSealingLimit::CheckpointSize,
                EpochSealingLimitAction::SealAfterAdmit,
            ),
            (
                EpochSealingLimit::CheckpointSize,
                EpochSealingLimitAction::RejectCandidate,
            ),
        ]);

        assert_eq!(
            verdict.checkpoint_size_action(),
            EpochSealingLimitAction::RejectCandidate
        );
    }

    #[test]
    fn test_merge_preserves_distinct_limits() {
        let mut tx_verdict = EpochSealingLimitVerdict::from_actions([(
            EpochSealingLimit::CheckpointSize,
            EpochSealingLimitAction::SealAfterAdmit,
        )]);
        let manifest_verdict = EpochSealingLimitVerdict::from_actions([(
            EpochSealingLimit::ManifestCount,
            EpochSealingLimitAction::RejectCandidate,
        )]);

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
    fn test_merge_keeps_most_restrictive_duplicate_limit() {
        let mut verdict = EpochSealingLimitVerdict::from_actions([(
            EpochSealingLimit::ManifestCount,
            EpochSealingLimitAction::SealAfterAdmit,
        )]);
        let stricter = EpochSealingLimitVerdict::from_actions([(
            EpochSealingLimit::ManifestCount,
            EpochSealingLimitAction::RejectCandidate,
        )]);

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
