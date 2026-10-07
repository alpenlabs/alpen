//! Spec selection follows the chain of the epoch it selects after.
//!
//! A fork can leave two summaries for one epoch. Selection must read the L1
//! range from the predecessor the parent epoch's own summary links to, not
//! from the canonical one, or it would count an enactment the parent epoch did
//! not process, or miss one it did.

use strata_acct_types::L1BlockRecord;
use strata_asm_common::AsmManifest;
use strata_checkpoint_types::EpochSummary;
use strata_identifiers::{
    Buf32, EpochCommitment, L1BlockCommitment, L1Height, OLBlockCommitment, OLBlockId,
};
use strata_ol_state_support_types::MemoryStateBaseLayer;
use strata_ol_state_types::{IStateAccessor, IStateAccessorMut, OLSpecId};
use strata_ol_state_types_v1::OLStateV1;
use strata_ol_stf_v1::test_utils::{
    make_checkpoint_predicate_enactment_manifest, make_empty_manifest, make_genesis_state,
};

use super::apply_checkpoint::MockChainWorkerContext;
use crate::{errors::WorkerError, state::select_spec_after};

/// The L1 height of `B`, the block whose manifest carries the enactment.
const B: L1Height = 5;

fn commitment(manifest: &AsmManifest) -> L1BlockCommitment {
    L1BlockCommitment::new(manifest.height(), *manifest.blkid())
}

fn terminal(slot: u64, byte: u8) -> OLBlockCommitment {
    OLBlockCommitment::new(slot, OLBlockId::from(Buf32::from([byte; 32])))
}

/// Returns a V1 state whose last L1 block is the last of `manifests`.
fn state_ending_on(manifests: &[AsmManifest]) -> MemoryStateBaseLayer<OLStateV1> {
    let mut state = make_genesis_state();
    for manifest in manifests {
        if manifest.height() > state.last_l1_height() {
            let record = L1BlockRecord::new(*manifest.blkid().as_ref(), [0; 32]);
            state.append_l1_block_rec(manifest.height(), record);
        }
    }
    assert_eq!(state.spec_versions().cur_spec(), OLSpecId::V1);
    state
}

/// Selects the spec after epoch 2, which ends on `B`, when the summary of
/// epoch 2 links to a predecessor ending on `linked_prev_last` and the
/// canonical predecessor, listed first, ends on `canonical_prev_last`.
fn select_after_fork(
    linked_prev_last: L1BlockCommitment,
    canonical_prev_last: L1BlockCommitment,
) -> Result<OLSpecId, WorkerError> {
    let manifests: Vec<AsmManifest> = (1..B)
        .map(|height| make_empty_manifest(height, 0))
        .chain([make_checkpoint_predicate_enactment_manifest(B, 1)])
        .collect();
    let b = commitment(manifests.last().expect("B's manifest"));
    let parent_state = state_ending_on(&manifests);

    let linked_prev = terminal(10, 1);
    let canonical_prev = terminal(11, 2);
    let parent = EpochCommitment::from_terminal(2, terminal(20, 3));
    let summary = |epoch, terminal, prev_terminal, new_l1| {
        EpochSummary::new(epoch, terminal, prev_terminal, new_l1, Buf32::zero())
    };

    let mut ctx = MockChainWorkerContext::new();
    ctx.epoch_summaries.insert(
        1,
        vec![
            summary(1, canonical_prev, terminal(0, 0), canonical_prev_last),
            summary(1, linked_prev, terminal(0, 0), linked_prev_last),
        ],
    );
    ctx.epoch_summaries.insert(
        2,
        vec![summary(2, parent.to_block_commitment(), linked_prev, b)],
    );
    ctx.manifests = manifests
        .into_iter()
        .map(|manifest| (manifest.height(), manifest))
        .collect();

    select_spec_after(&ctx, parent, &parent_state)
}

/// The parent epoch's own predecessor already ends on `B`, so the parent
/// processed no manifests and keeps V1, even though the canonical predecessor
/// ends earlier.
#[test]
fn test_selection_ignores_an_enactment_the_linked_predecessor_processed() {
    let b = commitment(&make_checkpoint_predicate_enactment_manifest(B, 1));
    let before_b = commitment(&make_empty_manifest(B - 1, 0));
    let spec = select_after_fork(b, before_b).expect("selects");
    assert_eq!(spec, OLSpecId::V1);
}

/// The parent epoch's own predecessor ends before `B`, so the parent
/// processed the enactment and V1 ends, even though the canonical predecessor
/// already ends on `B`.
#[test]
fn test_selection_counts_an_enactment_the_parent_processed() {
    let b = commitment(&make_checkpoint_predicate_enactment_manifest(B, 1));
    let before_b = commitment(&make_empty_manifest(B - 1, 0));
    let err = select_after_fork(before_b, b).expect_err("V1 ends at B");
    let WorkerError::UpgradeRequired(upgrade) = err else {
        panic!("expected an upgrade-required error, got {err}");
    };
    assert_eq!(upgrade.prev_spec(), OLSpecId::V1);
    assert_eq!(upgrade.enactment_l1_height(), B);
}
