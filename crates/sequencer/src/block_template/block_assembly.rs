use std::{thread, time};

use strata_asm_types::{L1BlockManifest, ProtocolOperation};
use strata_chainexec::MemStateAccessor;
use strata_chaintsn::context::StateAccessor;
use strata_checkpoint_types::Checkpoint;
use strata_common::retry::{
    policies::ExponentialBackoff, retry_with_backoff, DEFAULT_ENGINE_CALL_MAX_RETRIES,
};
use strata_consensus_logic::checkpoint_verification;
use strata_db::DbError;
use strata_eectl::{
    engine::{ExecEngineCtl, PayloadStatus},
    errors::EngineError,
    messages::{ExecPayloadData, PayloadEnv},
};
use strata_ol_chain_types::{
    ExecSegment, L1Segment, L2BlockAccessory, L2BlockBody, L2BlockBundle, L2BlockHeader, L2BlockId,
    L2Header,
};
use strata_ol_chainstate_types::Chainstate;
use strata_primitives::{
    buf::Buf32,
    params::{Params, RollupParams},
};
use strata_state::exec_update::construct_ops_from_deposit_intents;
use strata_storage::{CheckpointDbManager, L1BlockManager, NodeStorage};
use tracing::*;

use super::error::BlockAssemblyError as Error;

/// Get the total gas used by EL blocks from start of current epoch till prev_slot
fn get_total_gas_used_in_epoch(storage: &NodeStorage, prev_blkid: L2BlockId) -> Result<u64, Error> {
    let chainstate = storage
        .chainstate()
        .get_slot_write_batch_blocking(prev_blkid)?
        .ok_or(Error::Db(DbError::MissingSlotWriteBatch(prev_blkid)))?
        .into_toplevel();

    let prev_epoch = chainstate.prev_epoch();
    debug!(?prev_epoch);
    let epoch_start_slot = chainstate.prev_epoch().last_slot() + 1;
    let prev_header = storage
        .l2()
        .get_block_data_blocking(&prev_blkid)?
        .ok_or(Error::Db(DbError::MissingL2Block(prev_blkid)))?
        .header()
        .clone();
    let mut gas_used = 0;
    let prev_slot = prev_header.slot();

    let mut block_to_fetch = prev_blkid;
    for _ in epoch_start_slot..=prev_slot {
        let block: L2BlockBundle = storage
            .l2()
            .get_block_data_blocking(&block_to_fetch)?
            .ok_or(DbError::MissingL2Block(block_to_fetch))?;
        gas_used += block.accessory().gas_used();
        block_to_fetch = *block.header().parent();
    }

    // REVIEW: This doesn't work for the first block because prev_epoch_end_blkid = 0x00
    // let prev_epoch_end_blkid = *chainstate.prev_epoch().last_blkid();
    // assert_eq!(
    //     block_to_fetch, prev_epoch_end_blkid,
    //     "fetched blocks should end at the last block of the previous epoch"
    // );

    // TODO: cache
    Ok(gas_used)
}

/// Build contents for a new L2 block with the provided configuration.
/// Needs to be signed to be a valid L2Block.
// TODO use parent block chainstate
pub fn prepare_block(
    prev_block: L2BlockBundle,
    ts: u64,
    epoch_gas_limit: Option<u64>,
    storage: &NodeStorage,
    engine: &impl ExecEngineCtl,
    params: &Params,
) -> Result<(L2BlockHeader, L2BlockBody, L2BlockAccessory), Error> {
    let prev_blkid = prev_block.header().get_blockid();
    let prev_slot = prev_block.header().slot();
    debug!(%prev_slot, %prev_blkid, "preparing block");
    let l1man = storage.l1();
    let chsman = storage.chainstate();
    let ckptman = storage.checkpoint();

    let prev_global_sr = *prev_block.header().state_root();

    // Get the previous block's state
    // TODO make this get the prev block slot from somewhere more reliable in
    // case we skip slots
    let prev_blkid = prev_block.header().get_blockid();
    let prev_chstate = chsman
        .get_slot_write_batch_blocking(prev_blkid)?
        .ok_or(Error::MissingBlockChainstate(prev_blkid))?
        .into_toplevel();
    let first_block_of_epoch = prev_chstate.is_epoch_finishing();

    // Figure out the safe L1 blkid.
    // FIXME this is somewhat janky, should get it from the MMR
    let safe_l1_blkid = *prev_chstate.l1_view().safe_blkid();
    debug!(%safe_l1_blkid);

    // TODO Pull data from CSM state that we've observed from L1, including new
    // headers or any headers needed to perform a reorg if necessary.
    let l1_seg = prepare_l1_segment(
        &prev_chstate,
        l1man.as_ref(),
        ckptman.as_ref(),
        params.rollup(),
    )?;
    debug!(?l1_seg);

    let remaining_gas_limit = if first_block_of_epoch {
        epoch_gas_limit
    } else if let Some(epoch_gas_limit) = epoch_gas_limit {
        let gas_used = get_total_gas_used_in_epoch(storage, prev_blkid)?;
        Some(epoch_gas_limit.saturating_sub(gas_used))
    } else {
        None
    };

    // Prepare the execution segment, which right now is just talking to the EVM
    // but will be more advanced later.
    let slot = prev_slot + 1;

    let (exec_seg, block_acc) = prepare_exec_data(
        slot,
        ts,
        prev_blkid,
        prev_global_sr,
        &prev_chstate,
        safe_l1_blkid.into(),
        engine,
        params.rollup(),
        remaining_gas_limit,
    )?;

    // Assemble the body and fake header.
    let epoch = if first_block_of_epoch {
        prev_chstate.cur_epoch() + 1
    } else {
        prev_chstate.cur_epoch()
    };

    let body = L2BlockBody::new(l1_seg, exec_seg);
    let fake_stateroot = Buf32::zero();
    let fake_header = L2BlockHeader::new(slot, epoch, ts, prev_blkid, &body, fake_stateroot);

    // Execute the block to compute the new state root, then assemble the real header.
    // TODO do something with the write batch?  to prepare it in the database?
    let post_state = compute_post_state(prev_chstate, &fake_header, &body, params)?;

    // FIXME: invalid stateroot. Remove l2blockid from ChainState or stateroot from L2Block header.
    let new_state_root = post_state.compute_state_root();

    let header = L2BlockHeader::new(slot, epoch, ts, prev_blkid, &body, new_state_root);

    Ok((header, body, block_acc))
}

fn prepare_l1_segment(
    prev_chstate: &Chainstate,
    l1man: &L1BlockManager,
    ckptman: &CheckpointDbManager,
    params: &RollupParams,
) -> Result<L1Segment, Error> {
    // We aren't going to reorg, so we'll include blocks right up to the tip.
    let (cur_real_l1_height, _) = l1man
        .get_canonical_chain_tip()?
        .ok_or(Error::MissingTipBlock)?;
    let target_height = cur_real_l1_height.saturating_sub(params.l1_reorg_safe_depth as u64); // -1 to give some buffer for very short reorgs

    // Check to see if there's actually no blocks in the queue.  In that case we can just give
    // everything we know about.
    let cur_safe_height = prev_chstate.l1_view().safe_height();
    let cur_next_exp_height = prev_chstate.l1_view().next_expected_height();
    let l1_verified_block = prev_chstate.l1_view().header_vs().last_verified_block;
    trace!(%target_height, %cur_safe_height, %cur_next_exp_height, "figuring out which blocks to include in L1 segment");
    trace!(?l1_verified_block, "last verified L1 block");

    // If there isn't any new blocks to pull then we just give nothing.
    if target_height <= cur_next_exp_height {
        return Ok(L1Segment::new_empty(cur_safe_height));
    }

    // This is much simpler than it was before because I'm removing the ability
    // to handle reorgs properly.  This is fine, we'll re-add it later when we
    // make the L1 scan proof stuff more sophisticated.
    let mut payloads = Vec::new();
    let mut is_epoch_final_block = {
        if prev_chstate.cur_epoch() == 0 && !prev_chstate.is_epoch_finishing() {
            // no previous epoch, end epoch and send commitment immediately
            // including first L1 block available.
            true
        } else {
            // check for previous epoch's checkpoint in referenced l1 blocks
            // end epoch once previous checkpoint is seen
            false
        }
    };

    let prev_checkpoint = if prev_chstate.prev_epoch().is_null() {
        None
    } else {
        let prev_epoch = prev_chstate.prev_epoch().epoch();
        // previous checkpoint entry should exist in db
        let checkpoint = ckptman
            .get_checkpoint_blocking(prev_epoch)?
            .ok_or(Error::MissingCheckpoint(prev_epoch))?
            .checkpoint;

        Some(checkpoint)
    };

    // TODO: some way to avoid rechecking l1 blocks already checked during previous
    // block assemblies.
    for height in cur_next_exp_height..=target_height {
        let Some(rec) = try_fetch_manifest(height, l1man)? else {
            // If we are missing a record, then something is weird, but it would
            // still be safe to abort.
            warn!(%height, "missing expected L1 block during assembly");
            break;
        };

        if has_expected_checkpoint(&rec, prev_checkpoint.as_ref(), params) {
            // Found valid checkpoint for previous epoch. Should end current epoch.
            is_epoch_final_block = true;
        }

        payloads.push(rec);

        if is_epoch_final_block {
            // if epoch = 0, include first seen l1 block and create checkpoint
            // if epoch > 0, include till first seen block with correct checkpoint
            break;
        }
    }

    if !payloads.is_empty() {
        debug!(n = %payloads.len(), "have new L1 blocks to provide");
    }

    if is_epoch_final_block {
        let new_height = cur_safe_height + payloads.len() as u64;
        Ok(L1Segment::new(new_height, payloads))
    } else {
        Ok(L1Segment::new(cur_safe_height, Vec::new()))
    }
}

/// Check if block has the checkpoint we are expecting and checkpoint is valid.
fn has_expected_checkpoint(
    rec: &L1BlockManifest,
    expected_checkpoint: Option<&Checkpoint>,
    params: &RollupParams,
) -> bool {
    for op in rec.txs().iter().flat_map(|tx| tx.protocol_ops()) {
        let ProtocolOperation::Checkpoint(signed_checkpoint) = op else {
            continue;
        };

        // L1 Reader only adds a checkpoint ProtocolOperation if checkpoint signature valid.
        let checkpoint = signed_checkpoint.checkpoint();

        // Must have expected checkpoint.
        // Can be None before first checkpoint creation, where we dont care about this.
        let Some(expected) = expected_checkpoint else {
            continue;
        };

        // Must get expected checkpoint.
        if expected.commitment() != checkpoint.commitment() {
            warn!(got = ?checkpoint, ?expected, "got unexpected checkpoint");
            continue;
        }

        // Proof inside checkpoint must be valid.
        let proof_receipt = checkpoint_verification::construct_receipt(checkpoint);
        if let Err(err) = checkpoint_verification::verify_proof(checkpoint, &proof_receipt, params)
        {
            warn!(?err, blockid = %rec.blkid(), "checkpoint proof verification failed");
            continue;
        }

        // Found valid checkpoint for previous epoch. Should end current epoch.
        return true;
    }

    // Scanned all txn and did not find checkpoint
    false
}

#[expect(unused, reason = "used for fetching manifest")]
fn fetch_manifest(h: u64, l1man: &L1BlockManager) -> Result<L1BlockManifest, Error> {
    try_fetch_manifest(h, l1man)?.ok_or(Error::MissingL1BlockHeight(h))
}

fn try_fetch_manifest(h: u64, l1man: &L1BlockManager) -> Result<Option<L1BlockManifest>, Error> {
    Ok(l1man.get_block_manifest_at_height(h)?)
}

/// Prepares the execution segment for the block.
#[expect(clippy::too_many_arguments, reason = "used for preparing exec data")]
fn prepare_exec_data<E: ExecEngineCtl>(
    _slot: u64,
    timestamp: u64,
    prev_l2_blkid: L2BlockId,
    _prev_global_sr: Buf32,
    prev_chstate: &Chainstate,
    safe_l1_block: Buf32,
    engine: &E,
    params: &RollupParams,
    remaining_gas_limit: Option<u64>,
) -> Result<(ExecSegment, L2BlockAccessory), Error> {
    trace!("preparing exec payload");

    // Start preparing the EL payload.

    // construct el_ops by looking at chainstate
    let pending_deposits = prev_chstate.exec_env_state().pending_deposits();
    let el_ops = construct_ops_from_deposit_intents(pending_deposits, params.max_deposits_in_block);
    let payload_env = PayloadEnv::new(
        timestamp,
        prev_l2_blkid,
        safe_l1_block,
        el_ops,
        remaining_gas_limit,
    );

    // If the payload preparation fails, we can safely retry in the next iteration.
    // The fork-choice manager includes graceful shutdown logic to handle any
    // persistent issues that may arise.
    let key = retry_with_backoff(
        "engine_prepare_payload",
        DEFAULT_ENGINE_CALL_MAX_RETRIES,
        &ExponentialBackoff::default(),
        || engine.prepare_payload(payload_env.clone()),
    )?;
    trace!("submitted EL payload job, waiting for completion");

    // Wait 2 seconds for the block to be finished.
    // TODO Pull data from state about the new safe L1 hash, prev state roots,
    // etc. to assemble the payload env for this block.
    let wait = time::Duration::from_millis(100);
    let timeout = time::Duration::from_millis(3000);
    let Some((payload_data, gas_used)) = poll_status_loop(key, engine, wait, timeout)? else {
        return Err(Error::BlockAssemblyTimedOut);
    };
    trace!("finished EL payload job");

    // Reassemble it into an exec update.
    let exec_update = payload_data.exec_update().clone();
    let _applied_ops = payload_data.ops();
    let exec_seg = ExecSegment::new(exec_update);

    // And the accessory.
    let acc = L2BlockAccessory::new(payload_data.accessory_data().to_vec(), gas_used);

    Ok((exec_seg, acc))
}

fn poll_status_loop<E: ExecEngineCtl>(
    job: u64,
    engine: &E,
    wait: time::Duration,
    timeout: time::Duration,
) -> Result<Option<(ExecPayloadData, u64)>, EngineError> {
    let start = time::Instant::now();
    loop {
        // Sleep at the beginning since the first iter isn't likely to have it
        // ready.
        thread::sleep(wait);

        // Check the payload for the result.
        trace!(%job, "polling engine for completed payload");

        let payload = retry_with_backoff(
            "engine_get_payload_status",
            DEFAULT_ENGINE_CALL_MAX_RETRIES,
            &ExponentialBackoff::default(),
            || engine.get_payload_status(job),
        )?;

        if let PayloadStatus::Ready(pl, gas_used) = payload {
            return Ok(Some((pl, gas_used)));
        }

        // If we've waited too long now.
        if time::Instant::now() - start > timeout {
            warn!(%job, "payload build job timed out");
            break;
        }
    }

    Ok(None)
}

// TODO when we build the "block executor" logic we should shift this out
fn compute_post_state(
    prev_chstate: Chainstate,
    header: &L2BlockHeader,
    body: &L2BlockBody,
    params: &Params,
) -> Result<Chainstate, Error> {
    let mut state_accessor = MemStateAccessor::new(prev_chstate);
    strata_chaintsn::transition::process_block(&mut state_accessor, header, body, params.rollup())?;
    Ok(state_accessor.state_untracked().clone())
}
