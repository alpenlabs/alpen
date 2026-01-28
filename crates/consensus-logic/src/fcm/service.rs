use std::sync::Arc;

use anyhow::anyhow;
use bitcoin::absolute::Height;
use serde::Serialize;
use strata_csm_types::ClientState;
use strata_db_types::{traits::BlockStatus, DbError};
use strata_ledger_types::IStateAccessor;
use strata_ol_chain_types_new::OLBlock;
use strata_params::RollupParams;
use strata_primitives::{
    crypto::verify_schnorr_sig, Buf32, CredRule, EpochCommitment, L1BlockCommitment,
    OLBlockCommitment, OLBlockId,
};
use strata_service::{AsyncService, Response, Service, ServiceBuilder, ServiceMonitor};
use strata_status::{ChainSyncStatus, OLSyncStatusUpdate};
use strata_storage::OLBlockManager;
use strata_tasks::TaskExecutor;
use tokio::sync::mpsc::{channel as mpsc_channel, Sender};
use tracing::{debug, error, info, trace, warn};

use crate::{
    errors::Error,
    fcm::{input::FcmEvent, state::FcmState},
    init_fcm_service_state,
    message::ForkChoiceMessage,
    tip_update::{compute_tip_update, TipUpdate},
    FcmContext, FcmInput,
};

#[derive(Clone, Debug)]
pub struct FcmServiceHandle {
    fcm_tx: Sender<ForkChoiceMessage>,
    service_monitor: ServiceMonitor<FcmStatus>,
}

impl FcmServiceHandle {
    pub fn submit_chain_tip_msg_blocking(&self, msg: ForkChoiceMessage) -> bool {
        self.fcm_tx.blocking_send(msg).is_ok()
    }

    pub async fn submit_chain_tip_msg_async(&self, msg: ForkChoiceMessage) -> bool {
        self.fcm_tx.send(msg).await.is_ok()
    }

    pub fn fcm_status(&self) -> FcmStatus {
        self.service_monitor.get_current()
    }
}

pub async fn start_fcm_service(
    fcm_ctx: FcmContext,
    texec: Arc<TaskExecutor>,
) -> anyhow::Result<FcmServiceHandle> {
    let clstate_rx = fcm_ctx.status_channel().subscribe_checkpoint_state();

    // initialize fcm state
    let fcm_state = init_fcm_service_state(fcm_ctx).await?;

    let (fcm_tx, fcm_rx) = mpsc_channel::<ForkChoiceMessage>(64);
    let fcm_input = FcmInput::new(fcm_rx, clstate_rx);

    let service_monitor = ServiceBuilder::<FcmService, FcmInput>::new()
        .with_state(fcm_state)
        .with_input(fcm_input)
        .launch_async("fcm", texec.as_ref())
        .await?;
    Ok(FcmServiceHandle {
        service_monitor,
        fcm_tx,
    })
}

#[derive(Clone, Debug)]
pub struct FcmService;

#[derive(Clone, Debug, Serialize)]
pub struct FcmStatus;

impl Service for FcmService {
    type Msg = FcmEvent;
    type State = FcmState;
    type Status = FcmStatus;

    fn get_status(_s: &Self::State) -> Self::Status {
        FcmStatus
    }
}

impl AsyncService for FcmService {
    async fn on_launch(_state: &mut Self::State) -> anyhow::Result<()> {
        Ok(())
    }

    async fn before_shutdown(
        _state: &mut Self::State,
        _err: Option<&anyhow::Error>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn process_input(
        fcm_state: &mut Self::State,
        input: &Self::Msg,
    ) -> anyhow::Result<Response> {
        match input {
            FcmEvent::NewFcmMsg(m) => process_fc_message(m, fcm_state).await?,
            FcmEvent::NewStateUpdate(st) => handle_new_client_state(fcm_state, st).await?,
            FcmEvent::Abort => return Ok(Response::ShouldExit),
        };
        Ok(Response::Continue)
    }
}

async fn process_fc_message(
    msg: &ForkChoiceMessage,
    fcm_state: &mut FcmState,
) -> anyhow::Result<()> {
    let blk_db = fcm_state.ctx().storage().ol_block().clone();
    match msg {
        ForkChoiceMessage::NewBlock(blkid) => {
            strata_common::check_bail_trigger("fcm_new_block");

            let block_bundle = blk_db
                .get_block_data_async(*blkid)
                .await?
                .ok_or(Error::MissingL2Block(*blkid))?;

            let slot = block_bundle.header().slot();
            info!(%slot, %blkid, "processing new block");

            let ok = match handle_new_block(fcm_state, &block_bundle).await {
                Ok(v) => v,
                Err(e) => {
                    // Really we shouldn't emit this error unless there's a
                    // problem checking the block in general and it could be
                    // valid or invalid, but we're kinda sloppy with errors
                    // here so let's try to avoid crashing the FCM task?
                    error!(%slot, %blkid, "error processing block, interpreting as invalid\n{e:?}");
                    false
                }
            };

            let status = if ok {
                // check if any pending blocks can be finalized
                if let Err(err) = handle_epoch_finalization(fcm_state).await {
                    error!(%err, "failed to finalize epoch");
                }

                // Update status.
                let last_l1_blk = L1BlockCommitment::new(
                    Height::from_consensus(fcm_state.cur_ol_state().last_l1_height())
                        .expect("Invalid l1 height"),
                    *fcm_state.cur_ol_state().last_l1_blkid(),
                );

                let status = ChainSyncStatus {
                    tip: fcm_state.cur_best_block(),
                    prev_epoch: *fcm_state.cur_ol_state().previous_epoch(),
                    finalized_epoch: *fcm_state.chain_tracker().finalized_epoch(),
                    // FIXME this is a bit convoluted, could this be simpler?
                    safe_l1: last_l1_blk,
                };

                let update = OLSyncStatusUpdate::new(status);
                trace!(%blkid, "publishing new ol_state");
                fcm_state
                    .ctx()
                    .status_channel()
                    .update_ol_sync_status(update);

                BlockStatus::Valid
            } else {
                // Emit invalid block warning.
                warn!(%blkid, "rejecting invalid block");
                BlockStatus::Invalid
            };

            blk_db.set_block_status_async(*blkid, status).await?;
        }
    }

    Ok(())
}

async fn handle_new_client_state(fcm_state: &mut FcmState, cs: &ClientState) -> anyhow::Result<()> {
    let Some(new_fin_epoch) = cs.get_declared_final_epoch() else {
        debug!("got new CSM state, but finalized epoch still unset, ignoring");
        return Ok(());
    };

    info!(?new_fin_epoch, "got new finalized block");
    fcm_state.attach_epoch_pending_finalization(new_fin_epoch);

    match handle_epoch_finalization(fcm_state).await {
        Err(err) => {
            error!(%err, "failed to finalize epoch");
        }
        Ok(Some(finalized_epoch)) if finalized_epoch == new_fin_epoch => {
            debug!(?finalized_epoch, "finalized latest epoch");
        }
        Ok(Some(finalized_epoch)) => {
            debug!(?finalized_epoch, "finalized earlier epoch");
        }
        Ok(None) => {
            // there were no epochs that could be finalized
            warn!("did not finalize epoch");
        }
    };

    Ok(())
}

async fn handle_new_block(fcm_state: &mut FcmState, bundle: &OLBlock) -> anyhow::Result<bool> {
    let blk_db = fcm_state.ctx().storage().ol_block().clone();
    let slot = bundle.header().slot();
    let blkid = &bundle.header().compute_blkid();
    info!(%blkid, %slot, "handling new block");

    // First, decide if the block seems correctly signed and we haven't
    // already marked it as invalid.
    let check_res = check_ol_block_proposal_valid(blkid, bundle, fcm_state.ctx().params().rollup());
    if check_res.is_err() {
        // It's invalid, write that and return.
        return Ok(false);
    }

    // This stores the block output in the database, which lets us make queries
    // about it, at least until it gets reorged out by another block being
    // finalized.
    let bc = OLBlockCommitment::new(bundle.header().slot(), *blkid);
    let exec_ok = match fcm_state.ctx().chain_worker().try_exec_block_blocking(bc) {
        Ok(()) => true,
        Err(err) => {
            // TODO Need some way to distinguish an invalid block from a exec failure
            error!(%err, "try_exec_block failed");
            false
        }
    };

    if exec_ok {
        blk_db.set_block_status_blocking(*blkid, BlockStatus::Valid)?;
    } else {
        blk_db.set_block_status_blocking(*blkid, BlockStatus::Invalid)?;
        return Ok(false);
    }

    // Insert block into pending block tracker and figure out if we
    // should switch to it as a potential head.  This returns if we
    // created a new tip instead of advancing an existing tip.
    let cur_tip = *fcm_state.cur_best_block().blkid();
    let new_tip = fcm_state.chain_tracker_mut().attach_block(
        bundle.header().slot(),
        *blkid,
        *bundle.header().parent_blkid(),
    )?;

    if new_tip {
        debug!(?blkid, "created new branching tip");
    }

    // Now decide what the new tip should be and figure out how to get there.
    let best_block = pick_best_block(
        &cur_tip,
        fcm_state.chain_tracker().chain_tips_iter(),
        blk_db.as_ref(),
    )?;

    // TODO make configurable
    let depth = 100;

    let tip_update = compute_tip_update(&cur_tip, best_block, depth, fcm_state.chain_tracker())?;
    let Some(tip_update) = tip_update else {
        // In this case there's no change.
        return Ok(true);
    };

    let tip_blkid = *tip_update.new_tip();
    debug!(%tip_blkid, "have new tip, applying update");

    // Apply the reorg.
    let res = match apply_tip_update(tip_update, fcm_state, bundle).await {
        Ok(()) => {
            info!(%tip_blkid, "new chain tip");

            Ok(true)
        }

        Err(e) => {
            warn!(err = ?e, "failed to compute CL STF");

            // Specifically state transition errors we want to handle
            // specially so that we can remember to not accept the block again.
            if let Some(Error::InvalidStateTsn(inv_blkid, _)) = e.downcast_ref() {
                warn!(
                    ?blkid,
                    ?inv_blkid,
                    "invalid block on seemingly good fork, rejecting block"
                );

                Ok(false)
            } else {
                // Everything else we should fail on, signalling indeterminate
                // status for the block.
                Err(e)
            }
        }
    };

    res
}

/// Check if any pending epochs can be finalized.
/// If multiple are available, finalize the latest epoch that can be finalized.
/// Remove the finalized epoch and all earlier epochs from pending queue.
///
/// Note: Finalization in this context:
///     1. Update chaintip tracker's base block
///     2. Message execution engine to mark block corresponding to last block of this epoch as
///        finalized in the EE.
///
/// Return commitment to epoch that was finalized, if any.
async fn handle_epoch_finalization(
    fcm_state: &mut FcmState,
) -> anyhow::Result<Option<EpochCommitment>> {
    let Some((_idx, next_finalizable_epoch)) = fcm_state.find_latest_pending_finalizable_epoch()
    else {
        // no new blocks to finalize
        return Ok(None);
    };

    fcm_state.finalize_epoch(next_finalizable_epoch).await?;

    info!(?next_finalizable_epoch, "updated finalized tip");
    //trace!(?fin_report, "finalization report");
    // TODO do something with the finalization report?

    Ok(Some(next_finalizable_epoch))
}

/// Checks OL block's credential to ensure that it was authentically proposed.
pub fn check_ol_block_proposal_valid(
    _blkid: &OLBlockId,
    block: &OLBlock,
    params: &RollupParams,
) -> anyhow::Result<()> {
    // If it's not the genesis block, check that the block is correctly signed.
    if block.header().slot() > 0 {
        let sig = block
            .signed_header()
            .signature()
            .expect("signature not present");
        let msg: Buf32 = block.header().compute_blkid().into();
        let is_valid = match params.cred_rule {
            CredRule::Unchecked => true,
            CredRule::SchnorrKey(pubkey) => verify_schnorr_sig(sig, &msg, &pubkey),
        };
        if !is_valid {
            return Err(anyhow!("block creds check failed"));
        }
    }

    Ok(())
}

fn pick_best_block<'t>(
    cur_tip: &'t OLBlockId,
    tips_iter: impl Iterator<Item = &'t OLBlockId>,
    ol_block_mgr: &OLBlockManager,
) -> Result<&'t OLBlockId, Error> {
    let mut best_tip = cur_tip;
    let mut best_block = ol_block_mgr
        .get_block_data_blocking(*best_tip)?
        .ok_or(Error::MissingL2Block(*best_tip))?;

    // The implementation of this will only switch to a new tip if it's a higher
    // height than our current tip.  We'll make this more sophisticated in the
    // future if we have a more sophisticated consensus protocol.
    for other_tip in tips_iter {
        if other_tip == cur_tip {
            continue;
        }

        let other_block = ol_block_mgr
            .get_block_data_blocking(*other_tip)?
            .ok_or(Error::MissingL2Block(*other_tip))?;

        let best_header = best_block.header();
        let other_header = other_block.header();

        if other_header.slot() > best_header.slot() {
            best_tip = other_tip;
            best_block = other_block;
        }
    }

    Ok(best_tip)
}

async fn apply_tip_update(
    update: TipUpdate,
    fcm_state: &mut FcmState,
    bundle: &OLBlock,
) -> anyhow::Result<()> {
    let blk_db = fcm_state.ctx().storage().ol_state();
    match update {
        // Easy case.
        TipUpdate::ExtendTip(_cur, _new) => {
            // TODO: what's the relation between _new and bundle
            // Update the tip block in the FCM state.
            let blk_cmmt =
                OLBlockCommitment::new(bundle.header().slot(), bundle.header().compute_blkid());
            let ol_state = blk_db
                .get_toplevel_ol_state_async(blk_cmmt)
                .await?
                .ok_or(DbError::MissingStateInstance)?;

            fcm_state.update_tip_block(blk_cmmt, ol_state).await?;

            Ok(())
        }

        // Weird case that shouldn't normally happen.
        TipUpdate::LongExtend(_cur, mut intermediate, new) => {
            if intermediate.is_empty() {
                warn!("tip update is a LongExtend that should have been a ExtendTip");
            }

            // Push the new block onto the end and then use that list as the
            // blocks we're applying.
            intermediate.push(new);

            Ok(())
        }

        TipUpdate::Reorg(reorg) => {
            // See if we need to roll back recent changes.
            let pivot_blkid = reorg.pivot();
            let pivot_slot = fcm_state.get_block_slot(*pivot_blkid).await?;
            let pivot_block = OLBlockCommitment::new(pivot_slot, *pivot_blkid);

            // We probably need to roll back to an earlier block and update our
            // in-memory state first.
            if pivot_slot < fcm_state.cur_best_block().slot() {
                debug!(%pivot_blkid, %pivot_slot, "rolling back ol_state");
                revert_ol_state_to_block(&pivot_block, fcm_state).await?;
            } else {
                warn!("got a reorg that didn't roll back to an earlier pivot");
            }

            // TODO any cleanup?

            Ok(())
        }

        TipUpdate::Revert(_cur, new) => {
            let slot = fcm_state.get_block_slot(new).await?;
            let block = OLBlockCommitment::new(slot, new);
            revert_ol_state_to_block(&block, fcm_state).await?;
            Ok(())
        }
    }
}

/// Safely reverts the in-memory ol_state to a particular block, then rolls
/// back the writes on-disk.
async fn revert_ol_state_to_block(
    block: &OLBlockCommitment,
    fcm_state: &mut FcmState,
) -> anyhow::Result<()> {
    // Fetch the old state from the database and store in memory.  This
    // is also how  we validate that we actually *can* revert to this
    // block.
    let blkid = *block.blkid();
    let db = fcm_state.ctx().storage().ol_state();
    let new_state = db
        .get_toplevel_ol_state_blocking(*block)?
        .ok_or(Error::MissingBlockChainstate(blkid))?;
    let _ = fcm_state.update_tip_block(*block, new_state).await;

    // FIXME: Rollback the writes on the database that we no longer need.

    Ok(())
}
