use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::bail;
use bitcoin::{Block, BlockHash};
use bitcoind_async_client::traits::Reader;
use strata_btc_types::BlockHashExt;
use strata_config::btcio::ReaderConfig;
use strata_primitives::l1::{L1BlockCommitment, L1Height};
use strata_state::BlockSubmitter;
use strata_status::StatusChannel;
use strata_storage::{L1BlockManager, NodeStorage};
use tokio::time::sleep;
use tracing::*;

use super::event::L1Event;
use crate::{
    reader::{event::BlockData, handler::handle_bitcoin_event, state::ReaderState},
    status::{apply_status_updates, L1StatusUpdate},
    BtcioParams,
};

/// Context that encapsulates common items needed for L1 reader.
pub(crate) struct ReaderContext<R: Reader> {
    /// Bitcoin reader client
    pub client: Arc<R>,

    /// Storage
    pub storage: Arc<NodeStorage>,

    /// Config
    pub config: Arc<ReaderConfig>,

    /// Btcio params
    pub btcio_params: BtcioParams,

    /// Status transmitter
    pub status_channel: StatusChannel,
}

/// The main task that initializes the reader state and starts reading from bitcoin.
pub async fn bitcoin_data_reader_task<E: BlockSubmitter>(
    client: Arc<impl Reader>,
    storage: Arc<NodeStorage>,
    config: Arc<ReaderConfig>,
    btcio_params: BtcioParams,
    status_channel: StatusChannel,
    event_submitter: Arc<E>,
) -> anyhow::Result<()> {
    let target_next_block =
        calculate_target_next_block(storage.l1().as_ref(), btcio_params.genesis_l1_height())?;

    let ctx = ReaderContext {
        client,
        storage,
        config,
        btcio_params,
        status_channel,
    };
    do_reader_task(ctx, target_next_block, event_submitter.as_ref()).await
}

/// Calculates target next block to start polling l1 from.
fn calculate_target_next_block(
    l1_manager: &L1BlockManager,
    horz_height: L1Height,
) -> anyhow::Result<L1Height> {
    // TODO switch to checking the L1 tip in the consensus/client state
    let target_next_block = l1_manager
        .get_canonical_chain_tip()?
        .map(|(height, _)| height + 1)
        .unwrap_or(horz_height);
    assert!(target_next_block >= horz_height);
    Ok(target_next_block)
}

/// Inner function that actually does the reading task.
async fn do_reader_task<R: Reader>(
    ctx: ReaderContext<R>,
    target_next_block: L1Height,
    event_submitter: &impl BlockSubmitter,
) -> anyhow::Result<()> {
    info!(%target_next_block, "started L1 reader task!");

    let poll_dur = Duration::from_millis(ctx.config.client_poll_dur_ms as u64);
    let mut state = init_reader_state(&ctx, target_next_block).await?;
    let best_blkid = state.best_block();
    info!(%best_blkid, "initialized L1 reader state");

    loop {
        let mut status_updates: Vec<L1StatusUpdate> = Vec::new();

        match poll_for_new_blocks(&ctx, &mut state, &mut status_updates).await {
            Err(err) => {
                handle_poll_error(&err, &mut status_updates);
            }
            Ok(events) => {
                // handle events
                for ev in events {
                    handle_bitcoin_event(ev, &ctx, event_submitter).await?;
                }
            }
        };

        sleep(poll_dur).await;

        status_updates.push(L1StatusUpdate::LastUpdate(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        ));

        apply_status_updates(&status_updates, &ctx.status_channel).await;
    }
}

/// Handles errors encountered during polling.
fn handle_poll_error(err: &anyhow::Error, status_updates: &mut Vec<L1StatusUpdate>) {
    warn!(%err, "failed to poll Bitcoin client");
    status_updates.push(L1StatusUpdate::RpcError(err.to_string()));

    if let Some(reqwest_err) = err.downcast_ref::<reqwest::Error>() {
        if reqwest_err.is_connect() {
            status_updates.push(L1StatusUpdate::RpcConnected(false));
        }
        if reqwest_err.is_builder() {
            panic!("btcio: couldn't build the L1 client");
        }
    }
}

/// Inits the reader state by trying to backfill blocks up to a target height.
async fn init_reader_state<R: Reader>(
    ctx: &ReaderContext<R>,
    target_next_block: L1Height,
) -> anyhow::Result<ReaderState> {
    // Init the reader state using the blockid we were given, fill in a few blocks back.
    debug!(%target_next_block, "initializing reader state");
    let mut init_queue = VecDeque::new();

    let lookback = ctx.btcio_params.l1_reorg_safe_depth() as L1Height * 2;
    let client = ctx.client.as_ref();
    let genesis_height = ctx.btcio_params.genesis_l1_height();
    let pre_genesis = genesis_height.saturating_sub(1);

    // Do some math to figure out where our start and end are.
    let chain_info = client.get_blockchain_info().await?;
    let chain_tip = chain_info.blocks as L1Height;
    let start_height = target_next_block
        .saturating_sub(lookback)
        .max(pre_genesis)
        .min(chain_tip);
    let end_height = chain_tip.min(pre_genesis.max(target_next_block.saturating_sub(1)));
    debug!(%start_height, %end_height, "queried L1 client, have init range");

    // Loop through the range we've determined to be okay and pull the blocks we want to look back
    // through in.
    let mut real_cur_height = start_height;
    for height in start_height..=end_height {
        let blkid = client.get_block_hash(height as u64).await?;
        debug!(%height, %blkid, "loaded recent L1 block");
        init_queue.push_back(blkid);
        real_cur_height = height;
    }

    let epoch = ctx.status_channel.get_cur_chain_epoch().unwrap_or(0);

    // Note: Transaction filtering is no longer needed since the ASM STF handles
    // parsing L1 blocks and producing manifests with logs.
    let state = ReaderState::new(real_cur_height + 1, lookback as usize, init_queue, epoch);
    Ok(state)
}

/// Polls the chain to see if there's new blocks to look at, possibly reorging
/// if there's a mixup and we have to go back. Returns events corresponding to block and
/// transactions.
async fn poll_for_new_blocks<R: Reader>(
    ctx: &ReaderContext<R>,
    state: &mut ReaderState,
    status_updates: &mut Vec<L1StatusUpdate>,
) -> anyhow::Result<Vec<L1Event>> {
    let chain_info = ctx.client.get_blockchain_info().await?;
    status_updates.push(L1StatusUpdate::RpcConnected(true));
    let client_height = chain_info.blocks as L1Height;
    let fresh_best_block = chain_info.best_block_hash;

    if fresh_best_block == *state.best_block() {
        trace!("polled client, nothing to do");
        return Ok(vec![]);
    }

    let mut events = Vec::new();

    // First, check for a reorg if there is one.
    if let Some((pivot_height, pivot_blkid)) = find_pivot_block(ctx.client.as_ref(), state).await? {
        if pivot_height < state.best_block_idx() {
            info!(%pivot_height, %pivot_blkid, "found apparent reorg");
            let block = L1BlockCommitment::new(pivot_height, pivot_blkid.to_l1_block_id());
            state.rollback_to_height(pivot_height);

            // Return with the revert event immediately
            let revert_ev = L1Event::RevertTo(block);
            return Ok(vec![revert_ev]);
        }
    } else {
        // TODO make this case a bit more structured
        error!("unable to find common block with client chain, something is seriously wrong here!");
        bail!("things are broken with l1 reader");
    }

    debug!(%client_height, "have new blocks");

    // Now process each block we missed.
    let scan_start_height = state.next_height();
    for fetch_height in scan_start_height..=client_height {
        match fetch_and_process_block(ctx, fetch_height, state, status_updates).await {
            Ok((blkid, ev)) => {
                // Note: Checkpoint detection is now handled by the ASM STF via logs,
                // so we no longer update filter_config based on checkpoints here.
                events.push(ev);
                info!(%fetch_height, %blkid, "accepted new block");
            }
            Err(e) => {
                warn!(%fetch_height, err = %e, "failed to fetch new block");
                break;
            }
        };
    }

    Ok(events)
}

/// Finds the highest block index where we do agree with the node.  If we never
/// find one then we're really screwed.
async fn find_pivot_block(
    client: &impl Reader,
    state: &ReaderState,
) -> anyhow::Result<Option<(L1Height, BlockHash)>> {
    for (height, l1blkid) in state.iter_blocks_back() {
        // If at genesis, we can't reorg any farther.
        if height == 0 {
            return Ok(Some((height, *l1blkid)));
        }

        let queried_l1blkid = client.get_block_hash(height as u64).await?;
        trace!(%height, %l1blkid, %queried_l1blkid, "comparing blocks to find pivot");
        if queried_l1blkid == *l1blkid {
            return Ok(Some((height, *l1blkid)));
        }
    }

    Ok(None)
}

/// Fetches a block at given height, extracts relevant transactions and emits an [`L1Event`].
async fn fetch_and_process_block<R: Reader>(
    ctx: &ReaderContext<R>,
    height: L1Height,
    state: &mut ReaderState,
    status_updates: &mut Vec<L1StatusUpdate>,
) -> anyhow::Result<(BlockHash, L1Event)> {
    let block = ctx.client.get_block_at(height as u64).await?;
    let (evs, l1blkid) = process_block(ctx, state, status_updates, height, block).await?;

    // Insert to new block, incrementing cur_height.
    let _deep = state.accept_new_block(l1blkid);

    Ok((l1blkid, evs))
}

/// Processes a bitcoin Block to return corresponding `L1Event` and `BlockHash`.
async fn process_block<R: Reader>(
    _ctx: &ReaderContext<R>,
    state: &mut ReaderState,
    status_updates: &mut Vec<L1StatusUpdate>,
    height: L1Height,
    block: Block,
) -> anyhow::Result<(L1Event, BlockHash)> {
    let txs = block.txdata.len();

    // Note: Transaction indexing is no longer done here - the ASM STF handles
    // parsing L1 blocks and producing manifests with logs.
    let block_data = BlockData::new(height, block);

    let l1blkid = block_data.block().block_hash();

    trace!(%height, %l1blkid, %txs, "fetched block from client");

    status_updates.push(L1StatusUpdate::CurHeight(height));
    status_updates.push(L1StatusUpdate::CurTip(l1blkid.to_string()));

    let block_ev = L1Event::BlockData(block_data, state.epoch());

    Ok((block_ev, l1blkid))
}
