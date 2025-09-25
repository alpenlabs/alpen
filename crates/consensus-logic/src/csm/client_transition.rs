//! Core state transition function.

use bitcoin::Transaction;
use strata_asm_types::{L1BlockManifest, L1Tx, ProtocolOperation};
use strata_primitives::{
    batch::verify_signed_checkpoint_sig,
    l1::{L1BlockCommitment, L1BlockId},
    prelude::*,
};
use strata_state::{client_state::*, operation::*};
use strata_storage::NodeStorage;
use tracing::*;

use crate::{checkpoint_verification::verify_checkpoint, errors::*};

/// Interface for external context necessary specifically for transitioning.
pub trait EventContext {
    fn get_l1_block_manifest(&self, blockid: &L1BlockId) -> Result<L1BlockManifest, Error>;
    fn get_l1_block_manifest_at_height(&self, height: u64) -> Result<L1BlockManifest, Error>;
    fn get_client_state(&self, blockid: &L1BlockCommitment) -> Result<ClientState, Error>;
}

/// Event context using the main node storage interfaace.
#[derive(Clone)]
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug impls"
)]
pub struct StorageEventContext<'c> {
    storage: &'c NodeStorage,
}

impl<'c> StorageEventContext<'c> {
    pub fn new(storage: &'c NodeStorage) -> Self {
        Self { storage }
    }
}

impl EventContext for StorageEventContext<'_> {
    fn get_l1_block_manifest(&self, blockid: &L1BlockId) -> Result<L1BlockManifest, Error> {
        self.storage
            .l1()
            .get_block_manifest(blockid)?
            .ok_or(Error::MissingL1Block(*blockid))
    }

    fn get_l1_block_manifest_at_height(&self, height: u64) -> Result<L1BlockManifest, Error> {
        self.storage
            .l1()
            .get_block_manifest_at_height(height)?
            .ok_or(Error::MissingL1BlockHeight(height))
    }

    fn get_client_state(&self, blockid: &L1BlockCommitment) -> Result<ClientState, Error> {
        self.storage
            .client_state()
            .get_state_blocking(*blockid)?
            .ok_or(Error::MissingClientState(*blockid))
    }
}

// TODO(QQ): decouple checkpoint extraction, finalization and actions.
pub fn transition_client_state(
    cur_state: ClientState,
    cur_block: &L1BlockCommitment,
    next_block_mf: &L1BlockManifest,
    context: &impl EventContext,
    params: &Params,
) -> Result<(ClientState, Vec<SyncAction>), Error> {
    let rparams: &RollupParams = params.rollup();
    let genesis_height = rparams.genesis_l1_view.height();
    let next_block_height = next_block_mf.height();

    // Double check that we don't receive pre-genesis blocks.
    assert!(next_block_height >= genesis_height);

    // Asserts that we indeed have parent of the next_block set as a state.
    // The only case where this can be inaccurate is when cur_state is
    // a default pre-genesis mock, but it's handled by genesis.
    assert_eq!(next_block_mf.get_prev_blockid(), *cur_block.blkid());
    assert_eq!(cur_block.height() + 1, next_block_mf.height());

    let mut actions = vec![];

    // Structly speaking, here we rely on the fact that depth looks
    // at the canonical chain (and not on the fork).
    // Otherwise, we are screwed (because we query buried_block by height).
    let depth = rparams.l1_reorg_safe_depth as u64;
    let buried_height = next_block_mf.height().checked_sub(depth);
    let last_finalized_checkpoint = fetch_last_finalized_checkpoint(buried_height, context);

    // Extract the most recent checkpoint as of seen next_block_mf.
    // Also, populate sync actions.
    let recent_checkpoint = extract_recent_checkpoint(
        cur_state.get_last_checkpoint(),
        next_block_mf,
        rparams,
        &mut actions,
    )?;

    // Create the next client state.
    let next_state = ClientState::new(last_finalized_checkpoint, recent_checkpoint);

    let old_final_epoch = cur_state.get_declared_final_epoch();
    let new_final_epoch = next_state.get_declared_final_epoch();

    let new_declared = match (old_final_epoch, new_final_epoch) {
        (None, Some(new)) => Some(new),
        (Some(old), Some(new)) if new.epoch() > old.epoch() => Some(new),
        _ => None,
    };

    // Finalize the new epoch after the state transition (if any).
    if let Some(decl_epoch) = new_declared {
        actions.push(SyncAction::FinalizeEpoch(decl_epoch));
    }

    Ok((next_state, actions))
}

fn fetch_last_finalized_checkpoint(
    buried_height: Option<u64>,
    context: &impl EventContext,
) -> Option<L1Checkpoint> {
    if let Some(buried_h) = buried_height {
        let block = context.get_l1_block_manifest_at_height(buried_h).ok();
        if let Some(b) = block {
            if let Ok(cs) = context.get_client_state(&b.into()) {
                return cs.get_last_checkpoint();
            }
        }
    }
    None
}

fn extract_recent_checkpoint(
    prev_checkpoint: Option<L1Checkpoint>,
    block_mf: &L1BlockManifest,
    params: &RollupParams,
    sync_actions: &mut Vec<SyncAction>,
) -> Result<Option<L1Checkpoint>, Error> {
    let mut new_checkpoint = prev_checkpoint.clone();
    let height = block_mf.height();

    // Iterate through all of the protocol operations in all of the txs.
    // TODO split out each proto op handling into a separate function
    for tx in block_mf.txs() {
        for op in tx.protocol_ops() {
            if let ProtocolOperation::Checkpoint(signed_ckpt) = op {
                debug!(%height, "Obtained checkpoint in l1_block");
                // Before we do anything, check its signature.
                if !verify_signed_checkpoint_sig(signed_ckpt, &params.cred_rule) {
                    warn!(%height, "ignoring checkpointing with invalid signature");
                    continue;
                }

                let ckpt = signed_ckpt.checkpoint();

                // Now do the more thorough checks
                if verify_checkpoint(ckpt, prev_checkpoint.as_ref(), params).is_err() {
                    // If it's invalid then just print a warning and move on.
                    warn!(%height, "ignoring invalid checkpoint in L1 block");
                    continue;
                }

                let ckpt_ref = get_l1_reference(tx, *block_mf.blkid(), height)?;

                // Construct the state bookkeeping entry for the checkpoint.
                let l1ckpt = L1Checkpoint::new(
                    ckpt.batch_info().clone(),
                    *ckpt.batch_transition(),
                    ckpt_ref.clone(),
                );

                new_checkpoint = Some(l1ckpt);

                // Emit a sync action to update checkpoint entry in db
                sync_actions.push(SyncAction::UpdateCheckpointInclusion {
                    checkpoint: signed_ckpt.clone().into(),
                    l1_reference: ckpt_ref,
                });
            }
        }
    }

    Ok(new_checkpoint)
}

fn get_l1_reference(tx: &L1Tx, blockid: L1BlockId, height: u64) -> Result<CheckpointL1Ref, Error> {
    let btx: Transaction = tx.tx_data().try_into().map_err(|_e| {
        warn!(%height, "Invalid bitcoin transaction data in L1Tx");
        let msg = format!("Invalid bitcoin transaction data in L1Tx at height {height}");
        Error::Other(msg)
    })?;

    let txid = btx.compute_txid().into();
    let wtxid = btx.compute_wtxid().into();
    let l1_comm = L1BlockCommitment::new(height, blockid);
    Ok(CheckpointL1Ref::new(l1_comm, txid, wtxid))
}
