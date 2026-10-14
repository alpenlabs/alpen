//! Pure classification of one submitted checkpoint against the CSM's view of the ASM.

use strata_db_types::common::L1TxId;
use strata_identifiers::{Epoch, L1BlockCommitment, L1Height};
use strata_primitives::l1::is_l1_reorg_safe;

/// A checkpoint's reveal transaction in a canonical L1 block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MinedReveal {
    /// Transaction that was mined, which differs from the bundle's after a replacement.
    pub(crate) txid: L1TxId,

    /// Canonical block it was mined in.
    pub(crate) block: L1BlockCommitment,
}

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
    ConfirmedRejected(MinedReveal),
}

/// Classifies a checkpoint for `epoch` whose reveal is `mined`, if it is in a canonical block.
///
/// The ASM only accepts the checkpoint for `verified_epoch + 1`, so once the CSM has processed
/// the inclusion block, a verified epoch still below `epoch` means the ASM did not accept it.
/// Depth is measured against the CSM tip, not the L1 tip, so the ASM has always processed the
/// inclusion block before a rejection is reported. It uses the same reorg-safe rule as
/// checkpoint finality.
pub(crate) fn classify_submission(
    epoch: Epoch,
    mined: Option<MinedReveal>,
    csm_tip_height: L1Height,
    verified_epoch: Option<Epoch>,
    l1_reorg_safe_depth: u32,
) -> SubmissionVerdict {
    if verified_epoch.is_some_and(|verified| verified >= epoch) {
        return SubmissionVerdict::EpochAccepted;
    }
    let Some(mined) = mined else {
        return SubmissionVerdict::NotIncluded;
    };
    if is_l1_reorg_safe(mined.block.height(), csm_tip_height, l1_reorg_safe_depth) {
        SubmissionVerdict::ConfirmedRejected(mined)
    } else {
        SubmissionVerdict::AwaitingDepth
    }
}

#[cfg(test)]
mod tests {
    use strata_identifiers::{Buf32, L1BlockId, RBuf32};

    use super::*;

    const DEPTH: u32 = 3;

    fn mined_at(height: L1Height) -> MinedReveal {
        MinedReveal {
            txid: RBuf32::from([1; 32]),
            block: L1BlockCommitment::new(height, L1BlockId::from(Buf32::from([2; 32]))),
        }
    }

    #[test]
    fn accepted_epoch_is_never_rejected() {
        let verdict = classify_submission(5, Some(mined_at(100)), 200, Some(5), DEPTH);
        assert_eq!(verdict, SubmissionVerdict::EpochAccepted);

        // A later accepted epoch implies this one was accepted first.
        let verdict = classify_submission(5, Some(mined_at(100)), 200, Some(6), DEPTH);
        assert_eq!(verdict, SubmissionVerdict::EpochAccepted);
    }

    #[test]
    fn rejected_once_buried_at_depth() {
        // Inclusion at 100 under a tip at 102 is three confirmations.
        let verdict = classify_submission(5, Some(mined_at(100)), 102, Some(4), DEPTH);
        assert_eq!(verdict, SubmissionVerdict::ConfirmedRejected(mined_at(100)));
    }

    #[test]
    fn awaits_depth_one_block_short() {
        let verdict = classify_submission(5, Some(mined_at(100)), 101, Some(4), DEPTH);
        assert_eq!(verdict, SubmissionVerdict::AwaitingDepth);
    }

    #[test]
    fn awaits_depth_while_csm_lags_the_inclusion_block() {
        // The CSM has not processed the inclusion block yet, so the ASM has not judged it.
        let verdict = classify_submission(5, Some(mined_at(100)), 99, Some(4), DEPTH);
        assert_eq!(verdict, SubmissionVerdict::AwaitingDepth);
    }

    #[test]
    fn not_included_without_canonical_block() {
        let verdict = classify_submission(5, None, 200, Some(4), DEPTH);
        assert_eq!(verdict, SubmissionVerdict::NotIncluded);
    }

    #[test]
    fn rejected_before_any_accepted_checkpoint() {
        let verdict = classify_submission(1, Some(mined_at(100)), 102, None, DEPTH);
        assert_eq!(verdict, SubmissionVerdict::ConfirmedRejected(mined_at(100)));
    }

    #[test]
    fn zero_depth_requires_the_csm_to_process_the_block() {
        let verdict = classify_submission(5, Some(mined_at(100)), 100, Some(4), 0);
        assert_eq!(verdict, SubmissionVerdict::ConfirmedRejected(mined_at(100)));
        let verdict = classify_submission(5, Some(mined_at(100)), 99, Some(4), 0);
        assert_eq!(verdict, SubmissionVerdict::AwaitingDepth);
    }
}
