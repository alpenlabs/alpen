//! Top-level CL state transition logic.  This is largely stubbed off now, but
//! we'll replace components with real implementations as we go along.

use strata_checkpoint_types::Checkpoint;
use strata_ol_chain_types::{L2BlockBody, L2BlockHeader, L2Header};
use strata_params::RollupParams;
use strata_predicate::PredicateResult;
use strata_primitives::{epoch::EpochCommitment, l2::L2BlockCommitment};
use strata_state::prelude::StateQueue;
use tracing::warn;

use crate::{
    checkin::{process_l1_view_update, SegmentAuxData},
    context::StateAccessor,
    errors::TsnError,
    legacy::FauxStateCache,
};

/// Processes a block, making writes into the provided state cache.
///
/// The cache will eventually be written to disk.  This does not check the
/// block's credentials, it plays out all the updates a block makes to the
/// chain, but it will abort if there are any semantic issues that
/// don't make sense.
///
/// This operates on a state cache that's expected to be empty, may panic if
/// changes have been made, although this is not guaranteed.  Does not check the
/// `state_root` in the header for correctness, so that can be unset so it can
/// be use during block assembly.
pub fn process_block(
    state: &mut impl StateAccessor,
    header: &L2BlockHeader,
    body: &L2BlockBody,
    params: &RollupParams,
) -> Result<(), TsnError> {
    // Update basic bookkeeping.
    let prev_tip_slot = state.state_untracked().chain_tip_slot();
    let prev_tip_blkid = header.parent();
    state.set_slot(header.slot());
    state.set_prev_block(L2BlockCommitment::new(prev_tip_slot, *prev_tip_blkid));
    advance_epoch_tracking(state)?;
    // TODO: Fixme
    // if state.state_untracked().cur_epoch() != header.parent_header().epoch() {
    //     return Err(TsnError::MismatchEpoch(
    //         header.parent_header().epoch(),
    //         state.state_untracked().cur_epoch(),
    //     ));
    // }

    // Go through each stage and play out the operations it has.
    //
    // For now, we have to wrap these calls in some annoying bookkeeping while/
    // we transition to the new context traits.
    let cur_l1_height = state.state_untracked().l1_view().safe_height();
    let l1_prov = SegmentAuxData::new(cur_l1_height + 1, body.l1_segment());
    let mut faux_sc = FauxStateCache::new(state);
    let has_new_epoch = process_l1_view_update(&mut faux_sc, &l1_prov, params)?;

    // If we checked in with L1, then advance the epoch.
    if has_new_epoch {
        state.set_epoch_finishing_flag(true);
    }

    // After processing L1 segment, extract and add withdrawals from exec segment
    // TODO: remove ASAP
    for intent in body.exec_segment().update().output().withdrawals() {
        state
            .state_mut_untracked()
            .pending_withdraws_mut()
            .push_back(intent.clone());
    }

    Ok(())
}

/// Verify that the provided checkpoint proof is valid for the given params.
///
/// # Caution
///
/// If the checkpoint proof is empty, this function returns an `Ok(())`.
// FIXME this does not belong here, it should be in a more general module probably
pub fn verify_checkpoint_proof(
    checkpoint: &Checkpoint,
    rollup_params: &RollupParams,
) -> PredicateResult<()> {
    let checkpoint_idx = checkpoint.batch_info().epoch();
    let proof_receipt = checkpoint.construct_receipt();

    // FIXME: we are accepting empty proofs for now (devnet) to reduce dependency on the prover
    // infra.
    let is_empty_proof = proof_receipt.proof().is_empty();
    let allow_empty = rollup_params.proof_publish_mode.allow_empty();

    if is_empty_proof && allow_empty {
        warn!(%checkpoint_idx, "Verifying empty proof as correct");
        return Ok(());
    }

    rollup_params.checkpoint_predicate().verify_claim_witness(
        proof_receipt.public_values().as_bytes(),
        proof_receipt.proof().as_bytes(),
    )
}

/// Advances the epoch bookkeeping, if this is first slot of new epoch.
fn advance_epoch_tracking(state: &mut impl StateAccessor) -> Result<(), TsnError> {
    if !state.epoch_finishing_flag() {
        return Ok(());
    }

    let prev_block = state.state_untracked().prev_block();
    let cur_epoch = state.state_untracked().cur_epoch();
    let ended_epoch = EpochCommitment::new(cur_epoch, prev_block.slot(), *prev_block.blkid());
    state.set_prev_epoch(ended_epoch);
    state.set_cur_epoch(cur_epoch + 1);
    state.set_epoch_finishing_flag(false);

    // TODO: remove ASAP
    // Clear pending withdrawals at the start of each new epoch
    *state.state_mut_untracked().pending_withdraws_mut() = StateQueue::new_empty();

    Ok(())
}
