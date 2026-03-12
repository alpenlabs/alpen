use anyhow::Context;
use strata_asm_proto_checkpoint::subprotocol::CheckpointSubprotocol;
use strata_asm_txs_checkpoint::CHECKPOINT_SUBPROTOCOL_ID;
use strata_checkpoint_types::{BatchInfo, Checkpoint, CheckpointSidecar};
use strata_checkpoint_types_ssz::CheckpointTip;
use strata_csm_types::{CheckpointL1Ref, L1Checkpoint, SyncAction};
use strata_predicate::PredicateKey;
use strata_primitives::L1BlockCommitment;
use tracing::*;

use crate::{
    CsmWorkerState, processor::update_client_state_with_checkpoint, sync_actions::apply_action,
};

pub(crate) fn handle_checkpoint_v1_updates(
    state: &mut CsmWorkerState,
    asm_block: &L1BlockCommitment,
) -> anyhow::Result<()> {
    let asm_state = state
        .storage
        .asm()
        .get_state(*asm_block)
        .context("failed to get ASM state")?
        .context("ASM state not found")?;

    let new_checkpoint_state = asm_state
        .state()
        .find_section(CHECKPOINT_SUBPROTOCOL_ID)
        .context("checkpoint subprotocol section not found")?
        .try_to_state::<CheckpointSubprotocol>()
        .context("failed to deserialize checkpoint subprotocol state")?;

    let Some(old_state) = &state.last_checkpoint_state else {
        info!("no previous checkpoint state, initializing");
        state.last_checkpoint_state = Some(new_checkpoint_state);
        return Ok(());
    };

    if old_state == &new_checkpoint_state {
        info!("no changes to checkpoint state after processing L1 block");
        return Ok(());
    }

    // Clone values we need before mutably borrowing `state`.
    let old_tip = old_state.verified_tip;
    let old_seq_pred = old_state.sequencer_predicate.clone();
    let old_ckpt_pred = old_state.checkpoint_predicate.clone();

    if old_tip != new_checkpoint_state.verified_tip {
        handle_tip_update(
            state,
            asm_block,
            &old_tip,
            &new_checkpoint_state.verified_tip,
        )?;
    }

    if old_seq_pred != new_checkpoint_state.sequencer_predicate {
        handle_sequencer_predicate_update(&old_seq_pred, &new_checkpoint_state.sequencer_predicate);
    }

    if old_ckpt_pred != new_checkpoint_state.checkpoint_predicate {
        handle_checkpoint_predicate_update(
            &old_ckpt_pred,
            &new_checkpoint_state.checkpoint_predicate,
        );
    }

    state.last_checkpoint_state = Some(new_checkpoint_state);

    Ok(())
}

fn handle_tip_update(
    state: &mut CsmWorkerState,
    asm_block: &L1BlockCommitment,
    old_tip: &CheckpointTip,
    new_tip: &CheckpointTip,
) -> anyhow::Result<()> {
    let epoch = new_tip.epoch;
    info!(?old_tip, ?new_tip, %epoch, "checkpoint tip updated");

    let l1_start_height = old_tip.l1_height();
    let l1_start_manifest = state
        .storage
        .l1()
        .get_block_manifest_at_height(l1_start_height)
        .context("failed to get L1 block manifest for start height")?
        .context("L1 block manifest not found for start height")?;
    let l1_start = L1BlockCommitment::new(l1_start_height, *l1_start_manifest.blkid());

    let l1_end_height = new_tip.l1_height();
    let l1_end_manifest = state
        .storage
        .l1()
        .get_block_manifest_at_height(l1_end_height)
        .context("failed to get L1 block manifest for end height")?
        .context("L1 block manifest not found for end height")?;
    let l1_end = L1BlockCommitment::new(l1_end_height, *l1_end_manifest.blkid());

    let l1_range = (l1_start, l1_end);
    let l2_range = (*old_tip.l2_commitment(), *new_tip.l2_commitment());
    let batch = BatchInfo::new(epoch, l1_range, l2_range);

    let l1_ref = CheckpointL1Ref::new(*asm_block, Default::default(), Default::default());
    let l1_checkpoint = L1Checkpoint::new(batch.clone(), l1_ref.clone());

    // Update client state with the new checkpoint
    update_client_state_with_checkpoint(state, l1_checkpoint, epoch)?;

    // Store checkpoint entry in database
    let sync_action = SyncAction::UpdateCheckpointInclusion {
        checkpoint: Checkpoint::new(batch, Default::default(), CheckpointSidecar::new(vec![])),
        l1_reference: l1_ref,
    };
    apply_action(sync_action, &state.storage)?;

    state.last_processed_epoch = Some(epoch);

    Ok(())
}

fn handle_sequencer_predicate_update(
    old_sequencer_predicate: &PredicateKey,
    new_sequencer_predicate: &PredicateKey,
) {
    // TODO: handle the predicate key update
    // the sequencer should realize that it's predicate key was changed and should use the new
    // predicate to submit the checkpoint with, or if it doesn't have access to the new predicate
    // key it should stop all the sequencer duties
    info!(
        ?old_sequencer_predicate,
        ?new_sequencer_predicate,
        "sequencer predicate updated"
    );
}

fn handle_checkpoint_predicate_update(
    old_checkpoint_predicate: &PredicateKey,
    new_checkpoint_predicate: &PredicateKey,
) {
    // TODO: handle the predicate key update
    // the sequencer should use the new predicate to create the new proof with. there might be other
    // message passing mechanism that might be needed
    info!(
        ?old_checkpoint_predicate,
        ?new_checkpoint_predicate,
        "checkpoint predicate updated"
    );
}
