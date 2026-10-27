//! Pure classification of one submitted checkpoint against the CSM's view of the ASM.

use strata_db_types::common::L1TxId;
use strata_identifiers::{EpochCommitment, L1BlockCommitment, L1Height};
use strata_primitives::l1::is_l1_reorg_safe;

/// A checkpoint's reveal transaction in a canonical L1 block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MinedReveal {
    /// Transaction that was mined, which differs from the bundle's after a replacement.
    pub(crate) txid: L1TxId,

    /// Canonical block it was mined in.
    pub(crate) block: L1BlockCommitment,
}

/// What the ASM accepted for a submitted checkpoint's epoch, as the CSM saw it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EpochAcceptance {
    /// The ASM's verified epoch is still below the checkpoint's epoch.
    Pending,

    /// The ASM accepted `commitment` from transaction `txid`, observed in a canonical L1 block.
    Observed {
        commitment: EpochCommitment,
        txid: L1TxId,
    },

    /// The ASM accepted the epoch, but the node has no canonical L1 observation of which
    /// transaction it accepted.
    ///
    /// The genesis epoch is one case, since no checkpoint tip update covers it.
    Unobserved,
}

/// Where a submitted checkpoint stands relative to ASM acceptance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SubmissionVerdict {
    /// The ASM accepted this checkpoint's transaction, or accepted its epoch without the node
    /// observing which transaction it accepted.
    Accepted,

    /// The ASM accepted a different transaction for the epoch, and this checkpoint's transaction
    /// is not in a canonical L1 block, so it can no longer be accepted.
    Superseded,

    /// The checkpoint transaction is not in a canonical L1 block, and the ASM has not accepted
    /// the epoch.
    NotIncluded,

    /// The checkpoint transaction is in a canonical L1 block that is not yet buried at the
    /// reorg-safe depth under the CSM tip.
    AwaitingDepth,

    /// The checkpoint transaction is buried at the reorg-safe depth under the CSM tip and the
    /// ASM did not accept it.
    ConfirmedRejected(MinedReveal),
}

/// Classifies a checkpoint for `commitment` whose reveal is `mined`, if it is in a canonical
/// block.
///
/// The ASM accepts one transaction per epoch, so the checkpoint was rejected if the epoch is
/// still pending once the CSM has processed the inclusion block, or if the ASM accepted another
/// transaction for it: a different commitment, or the same commitment posted again. Depth is
/// measured against the CSM tip, not the L1 tip, so the ASM has always processed the inclusion
/// block before a rejection is reported. It uses the same reorg-safe rule as checkpoint finality.
pub(crate) fn classify_submission(
    commitment: EpochCommitment,
    mined: Option<MinedReveal>,
    acceptance: EpochAcceptance,
    csm_tip_height: L1Height,
    l1_reorg_safe_depth: u32,
) -> SubmissionVerdict {
    let accepted_another = match acceptance {
        EpochAcceptance::Pending => false,
        EpochAcceptance::Unobserved => return SubmissionVerdict::Accepted,
        EpochAcceptance::Observed {
            commitment: accepted,
            txid,
        } => {
            if accepted == commitment && mined.is_some_and(|mined| mined.txid == txid) {
                return SubmissionVerdict::Accepted;
            }
            true
        }
    };
    let Some(mined) = mined else {
        return if accepted_another {
            SubmissionVerdict::Superseded
        } else {
            SubmissionVerdict::NotIncluded
        };
    };
    if is_l1_reorg_safe(mined.block.height(), csm_tip_height, l1_reorg_safe_depth) {
        SubmissionVerdict::ConfirmedRejected(mined)
    } else {
        SubmissionVerdict::AwaitingDepth
    }
}

#[cfg(test)]
mod tests {
    use strata_identifiers::{Buf32, Epoch, L1BlockId, OLBlockId, RBuf32};

    use super::*;

    const DEPTH: u32 = 3;

    fn commitment(epoch: Epoch) -> EpochCommitment {
        EpochCommitment::new(epoch, 40, OLBlockId::from(Buf32::from([3; 32])))
    }

    fn mined_at(height: L1Height) -> MinedReveal {
        MinedReveal {
            txid: RBuf32::from([1; 32]),
            block: L1BlockCommitment::new(height, L1BlockId::from(Buf32::from([2; 32]))),
        }
    }

    fn observed(commitment: EpochCommitment, txid: L1TxId) -> EpochAcceptance {
        EpochAcceptance::Observed { commitment, txid }
    }

    #[test]
    fn own_accepted_transaction_is_never_rejected() {
        let mined = mined_at(100);
        let acceptance = observed(commitment(5), mined.txid);
        let verdict = classify_submission(commitment(5), Some(mined), acceptance, 200, DEPTH);
        assert_eq!(verdict, SubmissionVerdict::Accepted);
    }

    #[test]
    fn unobserved_acceptance_is_never_rejected() {
        let verdict = classify_submission(
            commitment(5),
            Some(mined_at(100)),
            EpochAcceptance::Unobserved,
            200,
            DEPTH,
        );
        assert_eq!(verdict, SubmissionVerdict::Accepted);
    }

    #[test]
    fn rejected_once_buried_at_depth() {
        // Inclusion at 100 under a tip at 102 is three confirmations.
        let verdict = classify_submission(
            commitment(5),
            Some(mined_at(100)),
            EpochAcceptance::Pending,
            102,
            DEPTH,
        );
        assert_eq!(verdict, SubmissionVerdict::ConfirmedRejected(mined_at(100)));
    }

    #[test]
    fn awaits_depth_one_block_short() {
        let verdict = classify_submission(
            commitment(5),
            Some(mined_at(100)),
            EpochAcceptance::Pending,
            101,
            DEPTH,
        );
        assert_eq!(verdict, SubmissionVerdict::AwaitingDepth);
    }

    #[test]
    fn awaits_depth_while_csm_lags_the_inclusion_block() {
        // The CSM has not processed the inclusion block yet, so the ASM has not judged it.
        let verdict = classify_submission(
            commitment(5),
            Some(mined_at(100)),
            EpochAcceptance::Pending,
            99,
            DEPTH,
        );
        assert_eq!(verdict, SubmissionVerdict::AwaitingDepth);
    }

    #[test]
    fn not_included_without_canonical_block() {
        let verdict =
            classify_submission(commitment(5), None, EpochAcceptance::Pending, 200, DEPTH);
        assert_eq!(verdict, SubmissionVerdict::NotIncluded);
    }

    #[test]
    fn zero_depth_requires_the_csm_to_process_the_block() {
        let pending = EpochAcceptance::Pending;
        let verdict = classify_submission(commitment(5), Some(mined_at(100)), pending, 100, 0);
        assert_eq!(verdict, SubmissionVerdict::ConfirmedRejected(mined_at(100)));
        let verdict = classify_submission(commitment(5), Some(mined_at(100)), pending, 99, 0);
        assert_eq!(verdict, SubmissionVerdict::AwaitingDepth);
    }

    #[test]
    fn rejected_when_the_asm_accepts_another_commitment() {
        let other = EpochCommitment::new(5, 44, OLBlockId::from(Buf32::from([4; 32])));
        let acceptance = observed(other, RBuf32::from([9; 32]));
        let verdict =
            classify_submission(commitment(5), Some(mined_at(100)), acceptance, 102, DEPTH);
        assert_eq!(verdict, SubmissionVerdict::ConfirmedRejected(mined_at(100)));

        let verdict =
            classify_submission(commitment(5), Some(mined_at(100)), acceptance, 101, DEPTH);
        assert_eq!(verdict, SubmissionVerdict::AwaitingDepth);
    }

    #[test]
    fn rejected_when_another_transaction_carries_the_accepted_commitment() {
        // A posting proven with the wrong artifact, then the same commitment re-proven and
        // accepted from a second transaction.
        let acceptance = observed(commitment(5), RBuf32::from([9; 32]));
        let verdict =
            classify_submission(commitment(5), Some(mined_at(100)), acceptance, 102, DEPTH);
        assert_eq!(verdict, SubmissionVerdict::ConfirmedRejected(mined_at(100)));
    }

    #[test]
    fn unmined_checkpoint_is_superseded_by_another_acceptance() {
        let acceptance = observed(commitment(5), RBuf32::from([9; 32]));
        let verdict = classify_submission(commitment(5), None, acceptance, 200, DEPTH);
        assert_eq!(verdict, SubmissionVerdict::Superseded);
    }
}
