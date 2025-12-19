//! Top-level CL state transition logic.  This is largely stubbed off now, but
//! we'll replace components with real implementations as we go along.

use strata_asm_types::{L1BlockManifest, ProtocolOperation};
use strata_checkpoint_types::{verify_signed_checkpoint_sig, Checkpoint, SignedCheckpoint};
use strata_ol_chain_types::{L2BlockBody, L2BlockHeader, L2Header};
use strata_ol_chainstate_types::StateCache;
use strata_params::RollupParams;
use strata_predicate::PredicateResult;
use strata_primitives::{epoch::EpochCommitment, l1::L1BlockId, l2::L2BlockCommitment};
use tracing::warn;

use crate::{
    checkin::{process_l1_view_update, SegmentAuxData},
    context::StateAccessor,
    errors::{OpError, TsnError},
    legacy::FauxStateCache,
    macros::*,
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

    Ok(())
}

#[allow(dead_code, clippy::allow_attributes, reason = "used for chaintsn")]
fn process_l1_block(
    state: &mut StateCache,
    block_mf: &L1BlockManifest,
    params: &RollupParams,
) -> Result<(), TsnError> {
    // Just iterate through every tx's operation and call out to the handlers for that.
    for tx in block_mf.txs() {
        let in_blkid = block_mf.blkid();
        for op in tx.protocol_ops() {
            // Try to process it, log a warning if there's an error.
            if let Err(e) = process_proto_op(state, block_mf, op, params) {
                warn!(?op, %in_blkid, %e, "invalid protocol operation");
            }
        }
    }

    Ok(())
}

fn process_proto_op(
    state: &mut StateCache,
    block_mf: &L1BlockManifest,
    op: &ProtocolOperation,
    params: &RollupParams,
) -> Result<(), OpError> {
    if let ProtocolOperation::Checkpoint(ckpt) = &op {
        process_l1_checkpoint(state, block_mf, ckpt, params)?;
    }

    Ok(())
}

fn process_l1_checkpoint(
    state: &mut StateCache,
    _src_block_mf: &L1BlockManifest,
    signed_ckpt: &SignedCheckpoint,
    params: &RollupParams,
) -> Result<(), OpError> {
    // If signature verification failed, return early and do **NOT** finalize epoch
    // Note: This is not an error because anyone is able to post data to L1
    if !verify_signed_checkpoint_sig(signed_ckpt, &params.cred_rule) {
        warn!("Invalid checkpoint: signature");
        return Err(OpError::InvalidSignature);
    }

    let ckpt = signed_ckpt.checkpoint(); // inner data
    let ckpt_epoch = ckpt.batch_transition().epoch;

    let _receipt = ckpt.construct_receipt();
    let fin_epoch = state.state().finalized_epoch();

    // Note: This is error because this is done by the sequencer
    if !is_checkpoint_null(ckpt, fin_epoch) && ckpt_epoch != fin_epoch.epoch() + 1 {
        error!(%ckpt_epoch, "Invalid checkpoint: proof for invalid epoch");
        return Err(OpError::EpochNotExtend);
    }

    // Also just check if the epoch numbers in batch transition and batch info match
    if ckpt_epoch != ckpt.batch_info().epoch() {
        return Err(OpError::MalformedCheckpoint);
    }
    verify_checkpoint_proof(ckpt, params).map_err(|_| OpError::InvalidProof)?;

    // Copy the epoch commitment and make it finalized.
    let _old_fin_epoch = state.state().finalized_epoch();
    let new_fin_epoch = ckpt.batch_info().get_epoch_commitment();

    // TODO go through and do whatever stuff we need to do now that's finalized

    state.set_finalized_epoch(new_fin_epoch);
    trace!(?new_fin_epoch, "observed finalized checkpoint");

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

/// Checks if the given checkpoint is null based on previous finalized epoch. A checkpoint is
/// considered null epoch if and only if it's epoch is 0 and the state's finalized epoch is 0.
/// Note that for null epoch we don't do continuity check.
fn is_checkpoint_null(ckpt: &Checkpoint, finalized_epoch: &EpochCommitment) -> bool {
    let ckpt_epoch = ckpt.batch_transition().epoch;
    // Checkpoint is null only if its epoch is 0 and the state's finalized epoch is 0.
    ckpt_epoch == 0 && finalized_epoch.epoch() == 0
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
    Ok(())
}

/// Checks the attested block IDs and parent blkid connections in new blocks.
// TODO unit tests
#[expect(dead_code, reason = "used for chaintsn")]
fn check_chain_integrity(
    cur_safe_height: u64,
    _cur_safe_blkid: &L1BlockId,
    new_height: u64,
    new_blocks: &[L1BlockManifest],
) -> Result<(), TsnError> {
    // Check that the heights match.
    if new_height != cur_safe_height + new_blocks.len() as u64 {
        // This is basically right for both cases.
        return Err(TsnError::SkippedBlock);
    }

    // Iterate over all the blocks in the new list and make sure they match.
    for (i, e) in new_blocks.iter().enumerate() {
        let height = cur_safe_height + i as u64;

        // Make sure the hash matches.
        let computed_id = L1BlockId::compute_from_header_buf(e.header());
        let attested_id = e.record().blkid();
        if computed_id != *attested_id {
            return Err(TsnError::L1BlockIdMismatch(
                height,
                *attested_id,
                computed_id,
            ));
        }

        // Make sure matches parent.
        // TODO FIXME I think my impl for parent_blkid is incorrect, fix this later
        /*let blk_parent = e.record().parent_blkid();
        if i == 0 {
            if blk_parent != *pivot_blkid {
                return Err(TsnError::L1BlockParentMismatch(h, blk_parent, *pivot_blkid));
            }
        } else {
            let parent_payload = &new_blocks[i - 1];
            let parent_id = parent_payload.record().blkid();
            if blk_parent != *parent_id {
                return Err(TsnError::L1BlockParentMismatch(h, blk_parent, *parent_id));
            }
        }*/
    }

    Ok(())
}
