//! Pure classification of one submitted checkpoint against the CSM's view of the ASM.

use strata_identifiers::{Epoch, L1Height};
use strata_primitives::l1::is_l1_reorg_safe;

/// Where a submitted checkpoint stands relative to ASM acceptance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SubmissionVerdict {
    /// The ASM's verified tip has reached the checkpoint's epoch.
    ///
    /// This says the epoch was accepted, not that this transaction was: a rejected posting
    /// followed by an accepted one for the same epoch lands here too.
    EpochAccepted,

    /// The checkpoint transaction is not in a canonical L1 block.
    NotIncluded,

    /// The checkpoint transaction is in a canonical L1 block that is not yet buried at the
    /// reorg-safe depth under the CSM tip.
    AwaitingDepth,

    /// The checkpoint transaction is buried at the reorg-safe depth under the CSM tip and the
    /// ASM has still not accepted its epoch.
    ConfirmedRejected,
}

/// Classifies a checkpoint for `epoch` whose transaction sits at `canonical_inclusion_height`.
///
/// The ASM only accepts the checkpoint for `verified_epoch + 1`, so once the CSM has processed
/// the inclusion block, a verified epoch still below `epoch` means the ASM did not accept it.
/// Depth is measured against the CSM tip, not the L1 tip, so the ASM has always processed the
/// inclusion block before a rejection is reported. It uses the same reorg-safe rule as
/// checkpoint finality.
pub(crate) fn classify_submission(
    epoch: Epoch,
    canonical_inclusion_height: Option<L1Height>,
    csm_tip_height: L1Height,
    verified_epoch: Option<Epoch>,
    l1_reorg_safe_depth: u32,
) -> SubmissionVerdict {
    if verified_epoch.is_some_and(|verified| verified >= epoch) {
        return SubmissionVerdict::EpochAccepted;
    }
    let Some(inclusion_height) = canonical_inclusion_height else {
        return SubmissionVerdict::NotIncluded;
    };
    if is_l1_reorg_safe(inclusion_height, csm_tip_height, l1_reorg_safe_depth) {
        SubmissionVerdict::ConfirmedRejected
    } else {
        SubmissionVerdict::AwaitingDepth
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEPTH: u32 = 3;

    #[test]
    fn accepted_epoch_is_never_rejected() {
        let verdict = classify_submission(5, Some(100), 200, Some(5), DEPTH);
        assert_eq!(verdict, SubmissionVerdict::EpochAccepted);

        // A later accepted epoch implies this one was accepted first.
        let verdict = classify_submission(5, Some(100), 200, Some(6), DEPTH);
        assert_eq!(verdict, SubmissionVerdict::EpochAccepted);
    }

    #[test]
    fn rejected_once_buried_at_depth() {
        // Inclusion at 100 under a tip at 102 is three confirmations.
        let verdict = classify_submission(5, Some(100), 102, Some(4), DEPTH);
        assert_eq!(verdict, SubmissionVerdict::ConfirmedRejected);
    }

    #[test]
    fn awaits_depth_one_block_short() {
        let verdict = classify_submission(5, Some(100), 101, Some(4), DEPTH);
        assert_eq!(verdict, SubmissionVerdict::AwaitingDepth);
    }

    #[test]
    fn awaits_depth_while_csm_lags_the_inclusion_block() {
        // The CSM has not processed the inclusion block yet, so the ASM has not judged it.
        let verdict = classify_submission(5, Some(100), 99, Some(4), DEPTH);
        assert_eq!(verdict, SubmissionVerdict::AwaitingDepth);
    }

    #[test]
    fn not_included_without_canonical_block() {
        let verdict = classify_submission(5, None, 200, Some(4), DEPTH);
        assert_eq!(verdict, SubmissionVerdict::NotIncluded);
    }

    #[test]
    fn rejected_before_any_accepted_checkpoint() {
        let verdict = classify_submission(1, Some(100), 102, None, DEPTH);
        assert_eq!(verdict, SubmissionVerdict::ConfirmedRejected);
    }

    #[test]
    fn zero_depth_requires_the_csm_to_process_the_block() {
        let verdict = classify_submission(5, Some(100), 100, Some(4), 0);
        assert_eq!(verdict, SubmissionVerdict::ConfirmedRejected);
        let verdict = classify_submission(5, Some(100), 99, Some(4), 0);
        assert_eq!(verdict, SubmissionVerdict::AwaitingDepth);
    }
}
