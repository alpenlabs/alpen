// TODO much of this should be converted over to just listening for FCM state updates

use std::sync::Arc;

use futures::StreamExt;
#[cfg(feature = "debug-utils")]
use strata_common::{check_and_pause_debug_async, WorkerType};
use strata_consensus_logic::{csm::message::ForkChoiceMessage, sync_manager::SyncManager};
use strata_primitives::epoch::EpochCommitment;
use strata_state::{
    block::L2BlockBundle, chain_state::Chainstate, client_state::ClientState, header::L2Header,
};
use strata_storage::NodeStorage;
use tracing::*;

use crate::{
    state::{self, L2SyncState},
    L2SyncError, SyncClient,
};

pub struct L2SyncContext<T: SyncClient> {
    client: T,
    storage: Arc<NodeStorage>,
    sync_manager: Arc<SyncManager>,
}

impl<T: SyncClient> L2SyncContext<T> {
    pub fn new(client: T, storage: Arc<NodeStorage>, sync_manager: Arc<SyncManager>) -> Self {
        Self {
            client,
            storage,
            sync_manager,
        }
    }
}

/// Initialize the sync state from the database and wait for the CSM to become ready.
async fn wait_until_ready_and_init_sync_state<T: SyncClient>(
    context: &L2SyncContext<T>,
) -> Result<L2SyncState, L2SyncError> {
    let (cstate_res, chainstate_res) = tokio::join!(
        wait_for_clientsate(&context.sync_manager),
        wait_for_chainstate(&context.sync_manager)
    );

    let cstate = cstate_res?;
    let chainstate = chainstate_res?;

    state::initialize_from_db(&cstate, &chainstate, context.storage.as_ref()).await
}

async fn wait_for_clientsate(sync_man: &SyncManager) -> Result<ClientState, L2SyncError> {
    let cstate = sync_man
        .status_channel()
        .wait_until_genesis()
        .await
        .map_err(|_| L2SyncError::ChannelClosed)?;

    Ok(cstate)
}

async fn wait_for_chainstate(sync_man: &SyncManager) -> Result<Chainstate, L2SyncError> {
    let mut service_init_rx = sync_man.status_channel().subscribe_service_init();

    // wait for FCM to be initialized. Dont want to start syncing from network while FCM is syncing
    // up from db.
    if service_init_rx
        .wait_for(|s| s.is_fcm_initialized())
        .await
        .is_err()
    {
        return Err(L2SyncError::ChannelClosed);
    }

    let mut chainstatus_rx = sync_man.status_channel().subscribe_chain_sync();

    let chainstate = chainstatus_rx
        .wait_for(Option::is_some)
        .await
        .map_err(|_| L2SyncError::ChannelClosed)?
        .as_ref()
        .expect("chainstate update should be present")
        .new_tl_chainstate()
        .as_ref()
        .clone();

    Ok(chainstate)
}

pub async fn sync_worker<T: SyncClient>(context: &L2SyncContext<T>) -> Result<(), L2SyncError> {
    let mut state = wait_until_ready_and_init_sync_state(context).await?;

    let mut chainsync_rx = context.sync_manager.status_channel().subscribe_chain_sync();
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = chainsync_rx.changed() => {
                let chainsync = chainsync_rx.borrow_and_update().clone();

                let Some(chainsync) = chainsync else {
                    continue;
                };
                let finalized_epoch = chainsync.new_status().finalized_epoch;
                handle_finalized_epoch(finalized_epoch, &mut state, context).await?;
            }

            _ = interval.tick() => {
                do_tick(&mut state, context).await?;
            }
            // maybe subscribe to new blocks on client instead of polling?
        }
    }
}

async fn handle_finalized_epoch<T: SyncClient>(
    fin_epoch: EpochCommitment,
    state: &mut L2SyncState,
    _context: &L2SyncContext<T>,
) -> Result<(), L2SyncError> {
    // on receiving new client update, update own finalized state

    if let Err(e) = handle_block_finalized(state, fin_epoch).await {
        error!(?fin_epoch, err = %e, "failed to handle newly finalized block");
    }

    Ok(())
}

async fn do_tick<T: SyncClient>(
    state: &mut L2SyncState,
    context: &L2SyncContext<T>,
) -> Result<(), L2SyncError> {
    // every fixed interval, try to sync with latest state of client
    let Ok(status) = context.client.get_sync_status().await else {
        // This should never *really* happen.
        warn!("failed to get client status");
        return Ok(());
    };

    if state.has_block(status.tip_block_id()) {
        // in sync with client
        return Ok(());
    }

    let start_slot = state.tip_height() + 1;
    let end_slot = status.tip_height(); // remote tip height

    let span = debug_span!("sync", %start_slot, %end_slot);

    if let Err(e) = sync_blocks_by_range(start_slot, end_slot, state, context)
        .instrument(span)
        .await
    {
        error!(%start_slot, %end_slot, err = ?e, "failed to make sync fetch");
    }

    Ok(())
}

async fn sync_blocks_by_range<T: SyncClient>(
    start_height: u64,
    end_height: u64,
    state: &mut L2SyncState,
    context: &L2SyncContext<T>,
) -> Result<(), L2SyncError> {
    debug!("syncing blocks by range");

    let block_stream = context.client.get_blocks_range(start_height, end_height);
    let mut block_stream = Box::pin(block_stream);

    while let Some(block) = block_stream.next().await {
        // NOTE: This is a noop if "debug-utils" flag is not turned on.
        #[cfg(feature = "debug-utils")]
        check_and_pause_debug_async(WorkerType::SyncWorker).await;

        handle_new_block(state, context, block).await?;
    }

    Ok(())
}

/// Process a new block received from the client.
///
/// The block is added to the unfinalized chain and the corresponding
/// messages are submitted to the fork choice manager.
///
/// If the parent block is missing, it will be fetched recursively
/// until we reach a known block in our unfinalized chain.
async fn handle_new_block<T: SyncClient>(
    state: &mut L2SyncState,
    context: &L2SyncContext<T>,
    block: L2BlockBundle,
) -> Result<(), L2SyncError> {
    let mut block = block;
    let mut fetched_blocks = vec![];

    loop {
        let block_id = block.header().get_blockid();
        debug!(block_id = ?block_id, height = block.header().blockidx(), "received new block");

        if state.has_block(&block_id) {
            warn!(block_id = ?block_id, "block already known");
            return Ok(());
        }

        let height = block.header().blockidx();
        if height <= state.finalized_height() {
            // got block on different fork than one we're finalized on
            // log error and ignore received blocks
            error!(height = height, block_id = ?block_id, "got block on incompatible fork");
            return Err(L2SyncError::WrongFork(block_id, height));
        }

        let parent_block_id = block.header().parent();

        fetched_blocks.push(block.clone());

        if state.has_block(parent_block_id) {
            break;
        }

        // parent block does not exist in our unfinalized chain
        // try to fetch it and continue
        let Some(parent_block) = context.client.get_block_by_id(parent_block_id).await? else {
            // block not found
            error!("parent block {parent_block_id} not found");
            return Err(L2SyncError::MissingBlock(*parent_block_id));
        };

        block = parent_block;
    }

    // send ForkChoiceMessage::NewBlock for all pending blocks in correct order
    while let Some(block) = fetched_blocks.pop() {
        state.attach_block(block.header())?;
        context
            .storage
            .l2()
            .put_block_data_async(block.clone())
            .await?;
        let block_idx = block.header().blockidx();
        debug!(%block_idx, "l2 sync: sending chain tip msg");
        context
            .sync_manager
            .submit_chain_tip_msg_async(ForkChoiceMessage::NewBlock(block.header().get_blockid()))
            .await;
        debug!(%block_idx, "l2 sync: sending chain tip sent");
    }
    Ok(())
}

async fn handle_block_finalized(
    state: &mut L2SyncState,
    new_finalized_epoch: EpochCommitment,
) -> Result<(), L2SyncError> {
    if state.finalized_blockid() == new_finalized_epoch.last_blkid() {
        return Ok(());
    }

    if !state.has_block(new_finalized_epoch.last_blkid()) {
        return Err(L2SyncError::MissingFinalized(
            *new_finalized_epoch.last_blkid(),
        ));
    };

    state.update_finalized_tip(new_finalized_epoch)?;

    Ok(())
}
