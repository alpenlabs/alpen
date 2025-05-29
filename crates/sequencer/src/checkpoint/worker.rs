//! worker to monitor chainstate and create checkpoint entries.

use std::sync::Arc;

use strata_db::{types::CheckpointEntry, DbError};
use strata_l1tx::filter::types::TxFilterConfig;
use strata_primitives::{
    self,
    epoch::EpochCommitment,
    hash::compute_borsh_hash,
    l1::{L1BlockCommitment, L1BlockManifest},
    l2::L2BlockCommitment,
    prelude::*,
};
use strata_state::{
    batch::{
        BatchInfo, BatchTransition, ChainstateRootTransition, EpochSummary,
        TxFilterConfigTransition,
    },
    block::L2BlockBundle,
    chain_state::Chainstate,
    header::*,
};
use strata_status::*;
use strata_storage::{CheckpointDbManager, L1BlockManager, L2BlockManager, NodeStorage};
use strata_tasks::ShutdownGuard;
use tokio::runtime::Handle;
use tracing::*;

use super::CheckpointHandle;
use crate::errors::Error;

/// Worker to monitor client state updates and create checkpoint entries
/// pending proof when previous proven checkpoint is finalized.
pub fn checkpoint_worker(
    shutdown: ShutdownGuard,
    status_ch: StatusChannel,
    params: Arc<Params>,
    storage: Arc<NodeStorage>,
    checkpoint_handle: Arc<CheckpointHandle>,
    rt: Handle,
) -> anyhow::Result<()> {
    let ckman = storage.checkpoint();

    let mut chs_rx = SyncReceiver::new(status_ch.subscribe_chain_sync(), rt);

    //let rollup_params_commitment = params.rollup().compute_hash();

    // FIXME this should have special handling for genesis
    let mut last_saved_epoch = ckman.get_last_checkpoint_blocking()?.unwrap_or_default();

    loop {
        if shutdown.should_shutdown() {
            warn!("received shutdown signal");
            break;
        }

        // Wait for a new update.
        if chs_rx.changed().is_err() {
            break;
        }

        // Get it if there is one.
        let update = chs_rx.borrow_and_update();
        let Some(update) = update.as_ref() else {
            trace!("received new chain sync status but was still unset, ignoring");
            continue;
        };

        let cur_epoch = update.new_status().cur_epoch();
        debug!(%last_saved_epoch, %cur_epoch, "checkpoint got new chainstate update");

        // Again check if we should shutdown, just in case.
        if shutdown.should_shutdown() {
            warn!("received shutdown signal");
            break;
        }

        // Fetch the epochs that seem ready to have checkpoints generated.  We
        // don't actually use the update for this, it's just a signal to check.
        // Maybe that could be simplified?

        let ready_epochs = find_ready_checkpoints(last_saved_epoch, ckman)?;

        if !ready_epochs.is_empty() {
            let n_ready = ready_epochs.len();
            trace!(%last_saved_epoch, %n_ready, "found epochs ready for checkpoint");
        } else {
            trace!("no new epochs ready for checkpoint");
        }

        for ec in ready_epochs {
            let Some(summary) = ckman.get_epoch_summary_blocking(ec)? else {
                warn!(?ec, "epoch seemed ready but summary was missing, ignoring");
                continue;
            };

            let terminal_blkid = ec.last_blkid();
            let epoch = ec.epoch();
            info!(%epoch, %terminal_blkid, "generating checkpoint for epoch");

            // If this errors we should crash probably.
            handle_ready_epoch(
                &summary,
                storage.as_ref(),
                checkpoint_handle.as_ref(),
                params.rollup(),
            )?;

            last_saved_epoch = epoch;
        }
    }
    Ok(())
}

/// Finds any epoch after a given epoch number that have been inserted but we
/// haven't inserted checkpoint entries for.
fn find_ready_checkpoints(
    from_epoch: u64,
    ckman: &CheckpointDbManager,
) -> anyhow::Result<Vec<EpochCommitment>> {
    let epoch_at = from_epoch; // TODO make this +1 after we fix genesis
    let Some(last_ready_epoch) = ckman.get_last_summarized_epoch_blocking()? else {
        warn!("no epoch summaries have been written, skipping");
        return Ok(Vec::new());
    };

    trace!(%from_epoch, %last_ready_epoch, "fetching epoch commitments");

    let mut epochs = Vec::new();
    for i in epoch_at..=last_ready_epoch {
        let commitments = ckman.get_epoch_commitments_at_blocking(i)?;

        if commitments.is_empty() {
            warn!(epoch = %i, "thought there was an epoch summary here, moving on");
            continue;
        }

        if commitments.len() > 1 {
            let ignored_count = commitments.len() - 1;
            warn!(epoch = %i, %ignored_count, "ignoring some summaries at epoch");
        }

        // TODO: this is not actually correct although should be fine under the assumption of
        // no-reorg.
        // This function should be passed the last accepted checkpoint and this should be the
        // commitment that continues the last checkpoint.
        let ec = commitments[0];
        if ckman.get_checkpoint_blocking(ec.epoch())?.is_none() {
            trace!(epoch = %i, "found epoch ready to checkpoint");
            epochs.push(ec);
        }
    }

    Ok(epochs)
}

fn handle_ready_epoch(
    epoch_summary: &EpochSummary,
    storage: &NodeStorage,
    ckhandle: &CheckpointHandle,
    params: &RollupParams,
) -> anyhow::Result<()> {
    let epoch = epoch_summary.epoch();
    let new_l1 = epoch_summary.new_l1();

    info!(%epoch, ?new_l1, "preparing checkpoint data");

    // REALLY make sure we don't already have checkpoint for the epoch.
    if ckhandle.get_checkpoint_blocking(epoch)?.is_some() {
        warn!(%epoch, "already have checkpoint for epoch, aborting preparation");
        return Ok(());
    }

    let cpd = create_checkpoint_prep_data_from_summary(epoch_summary, storage, params)?;

    // Commented out version of this that avoids a crash if it fails.  Was used
    // in troubleshooting.  But do we really need it?
    /*let cpd = match create_checkpoint_prep_data_from_summary(epoch_summary, storage, params) {
        Ok(cpd) => cpd,
        Err(e) => {
            error!("failed to generate checkpoint prep data, this shouldn't be possible if we generated the epoch authentically");
            error!("backtrace:\n{e}");

            // We don't want to crash.
            return Ok(());
        }
    };*/

    // sanity check
    assert_eq!(
        cpd.info.epoch(),
        epoch_summary.epoch(),
        "ckptworker: epoch mismatch in checkpoint preparation"
    );

    // else save a pending proof checkpoint entry
    debug!(%epoch, "saving unproven checkpoint");
    let entry = CheckpointEntry::new_pending_proof(cpd.info, cpd.tsn, &cpd.chainstate);
    if let Err(e) = ckhandle.put_checkpoint_and_notify_blocking(epoch, entry) {
        warn!(%epoch, err = %e, "failed to save checkpoint");
    }

    Ok(())
}

/// Container structure for convenience.
struct CheckpointPrepData {
    info: BatchInfo,
    tsn: BatchTransition,
    chainstate: Chainstate,
}

impl CheckpointPrepData {
    const fn new(info: BatchInfo, tsn: BatchTransition, chainstate: Chainstate) -> Self {
        Self {
            info,
            tsn,
            chainstate,
        }
    }
}

/// Creates the CPD for a completed epoch from an epoch summary, if possible.
fn create_checkpoint_prep_data_from_summary(
    summary: &EpochSummary,
    storage: &NodeStorage,
    params: &RollupParams,
) -> anyhow::Result<CheckpointPrepData> {
    let l1man = storage.l1();
    let l2man = storage.l2();
    let chsman = storage.chainstate();

    let epoch = summary.epoch();
    let is_genesis_epoch = epoch == 0;

    // There's some special handling we have to do if we're the genesis epoch.
    let prev_summary = if !is_genesis_epoch {
        let ec = summary.get_prev_epoch_commitment().unwrap();
        let ps = storage
            .checkpoint()
            .get_epoch_summary_blocking(ec)?
            .ok_or(Error::MissingEpochSummary(ec))?;
        Some(ps)
    } else {
        None
    };

    // Determine the ranges for each of the fields we commit to.
    let l1_start_height = if let Some(ps) = prev_summary {
        ps.new_l1().height() + 1
    } else {
        params.genesis_l1_height + 1
    };

    // Reconstruct the L1 range.
    let l1_start_mf = fetch_l1_block_manifest(l1_start_height, l1man)?;
    let l1_start_block = L1BlockCommitment::new(l1_start_height, *l1_start_mf.blkid());
    let l1_range = (l1_start_block, *summary.new_l1());

    // Now just pull out the data about the blocks from the transition here.
    //
    // There's a slight weirdness here.  The "range" refers to the first block
    // of the epoch, but the "transition" refers to the final state (ie last
    // block, for now) of the previous epoch.
    let l2_blocks = fetch_epoch_l2_headers(summary, l2man)?;
    let first_block = l2_blocks.first().unwrap();
    let last_block = l2_blocks.last().unwrap();
    let initial_l2_commitment =
        L2BlockCommitment::new(first_block.slot(), first_block.get_blockid());
    let l2_range = (initial_l2_commitment, *summary.terminal());

    // Initial state is the state before applying the first block
    let initial_state_height = first_block.slot() - 1;
    let initial_state = chsman
        .get_toplevel_chainstate_blocking(initial_state_height)?
        .ok_or(Error::MissingIdxChainstate(initial_state_height))?
        .to_chainstate();
    let l2_initial_state = initial_state.compute_state_root();

    let final_state_height = last_block.slot();
    let final_state = chsman
        .get_toplevel_chainstate_blocking(final_state_height)?
        .ok_or(Error::MissingIdxChainstate(final_state_height))?
        .to_chainstate();
    let l2_final_state = final_state.compute_state_root();

    let mut tx_filters = TxFilterConfig::derive_from(params)
        .expect("tx filter derivation from rollup params should not fail");

    // In the first (epoch 0), there's no changes to the TxFilterConfig
    // It is only at the end of epoch 1, that the TxFilterConfig will be changed
    let tx_filters_transition = if epoch < 1 {
        let tx_filters_hash = compute_borsh_hash(&tx_filters);
        TxFilterConfigTransition {
            pre_config_hash: tx_filters_hash,
            post_config_hash: tx_filters_hash,
        }
    } else {
        // Chainstate based on which the tx filter rules are updated
        let prev_checkpoint = storage
            .checkpoint()
            .get_checkpoint_blocking(epoch - 1)?
            .expect("checkpoint for the previous epoch must be valid");

        // Sanity check
        assert_eq!(
            prev_checkpoint
                .checkpoint
                .batch_transition()
                .chainstate_transition
                .post_state_root,
            l2_initial_state,
            "Chain state must continue from the last epoch"
        );

        // The TxFilterConfig for this epoch must be based on the TxFilterConfig derived at the end
        // of previous epoch
        let initial_tx_filters_config_hash = prev_checkpoint
            .checkpoint
            .batch_transition()
            .tx_filters_transition
            .post_config_hash;

        // The TxFilterConfig for the next epoch is derived at the end of this epoch. This is based
        // on the Chainstate that was posted on previous epoch, and included in L1Segment in this
        // epoch.
        let prev_chainstate: Chainstate =
            borsh::from_slice(prev_checkpoint.checkpoint.sidecar().chainstate())
                .expect("valid chainstate must be posted");

        tx_filters.update_from_chainstate(&prev_chainstate);
        let final_tx_filters_config_hash = compute_borsh_hash(&tx_filters);

        TxFilterConfigTransition {
            pre_config_hash: initial_tx_filters_config_hash,
            post_config_hash: final_tx_filters_config_hash,
        }
    };

    let chainstate_transition = ChainstateRootTransition {
        pre_state_root: l2_initial_state,
        post_state_root: l2_final_state,
    };

    let new_transition = BatchTransition {
        epoch: summary.epoch(),
        chainstate_transition,
        tx_filters_transition,
    };

    let new_batch_info = BatchInfo::new(summary.epoch(), l1_range, l2_range);

    Ok(CheckpointPrepData::new(
        new_batch_info,
        new_transition,
        final_state,
    ))
}

fn fetch_epoch_l2_headers(
    summary: &EpochSummary,
    l2man: &L2BlockManager,
) -> anyhow::Result<Vec<L2BlockHeader>> {
    let mut headers = Vec::new();

    let terminal = fetch_l2_block(summary.terminal().blkid(), l2man)?;
    headers.push(terminal.header().header().clone());

    // The loop keeps fetching the current header's parent block and extending
    // the list with it.
    //
    // The break conditions are a little weird so we use a bare `loop`.
    loop {
        // let limit = 5000; // TODO make a const
        // TODO: we need some way to limit L2 blocks in an epoch, we can't just error like this
        // if headers.len() >= limit {
        //     return Err(Error::MalformedEpoch(summary.get_epoch_commitment()).into());
        // }

        let cur = headers.last().unwrap();
        let cur_parent = cur.parent();

        // If we're at the first block we can just exit.
        // Checkpoint 0 range starts from block 1.
        if cur.slot() == 1 {
            break;
        }

        // If the current block's parent is the previous epoch's terminal block,
        // we can just break.
        if cur_parent == summary.prev_terminal().blkid() {
            break;
        }

        // Otherwise, just fetch the block and attach it.
        let block = fetch_l2_block(cur_parent, l2man)?;
        headers.push(block.header().header().clone());
    }

    // Also reverse the headers list so that earlier blocks are at the beginning.
    headers.reverse();

    Ok(headers)
}

fn fetch_l2_block(blkid: &L2BlockId, l2man: &L2BlockManager) -> anyhow::Result<L2BlockBundle> {
    Ok(l2man
        .get_block_data_blocking(blkid)?
        .ok_or(Error::MissingL2Block(*blkid))?)
}

fn fetch_l1_block_manifest(height: u64, l1man: &L1BlockManager) -> anyhow::Result<L1BlockManifest> {
    Ok(l1man
        .get_block_manifest_at_height(height)?
        .ok_or(DbError::MissingL1Block(height))?)
}
