use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::bail;
use bitcoin::{Block, BlockHash, CompactTarget};
use bitcoind_async_client::traits::Reader;
use strata_config::btcio::ReaderConfig;
use strata_l1tx::{
    filter::{indexer::index_block, types::TxFilterConfig},
    messages::RelevantTxEntry,
};
use strata_primitives::{
    l1::{
        get_relative_difficulty_adjustment_height, BtcParams, HeaderVerificationState,
        L1BlockCommitment, L1BlockId, TIMESTAMPS_FOR_MEDIAN,
    },
    params::{GenesisL1View, Params},
};
use strata_state::sync_event::BlockSubmitter;
use strata_status::StatusChannel;
use strata_storage::{L1BlockManager, NodeStorage};
use tracing::*;

use super::event::L1Event;
use crate::{
    reader::{
        event::BlockData,
        handler::handle_bitcoin_event,
        state::ReaderState,
        tx_indexer::ReaderTxVisitorImpl,
        utils::{find_checkpoint_in_event, find_last_checkpoint_chainstate},
    },
    status::{apply_status_updates, L1StatusUpdate},
};

/// Context that encapsulates common items needed for L1 reader.
pub(crate) struct ReaderContext<R: Reader> {
    /// Bitcoin reader client
    pub client: Arc<R>,

    /// Storage
    pub storage: Arc<NodeStorage>,

    /// Config
    pub config: Arc<ReaderConfig>,

    /// Params
    pub params: Arc<Params>,

    /// Status transmitter
    pub status_channel: StatusChannel,
}

/// The main task that initializes the reader state and starts reading from bitcoin.
pub async fn bitcoin_data_reader_task<E: BlockSubmitter>(
    client: Arc<impl Reader>,
    storage: Arc<NodeStorage>,
    config: Arc<ReaderConfig>,
    params: Arc<Params>,
    status_channel: StatusChannel,
    event_submitter: Arc<E>,
) -> anyhow::Result<()> {
    let target_next_block =
        calculate_target_next_block(storage.l1().as_ref(), params.rollup().horizon_l1_height)?;

    let ctx = ReaderContext {
        client,
        storage,
        config,
        params,
        status_channel,
    };
    do_reader_task(ctx, target_next_block, event_submitter.as_ref()).await
}

/// Calculates target next block to start polling l1 from.
fn calculate_target_next_block(
    l1_manager: &L1BlockManager,
    horz_height: u64,
) -> anyhow::Result<u64> {
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
    target_next_block: u64,
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

        tokio::time::sleep(poll_dur).await;

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
    target_next_block: u64,
) -> anyhow::Result<ReaderState> {
    // Init the reader state using the blockid we were given, fill in a few blocks back.
    debug!(%target_next_block, "initializing reader state");
    let mut init_queue = VecDeque::new();

    let lookback = ctx.params.rollup().l1_reorg_safe_depth as usize * 2;
    let client = ctx.client.as_ref();
    let genesis_height = ctx.params.rollup().genesis_l1_view.height();
    let pre_genesis = genesis_height.saturating_sub(1);
    let target = target_next_block as i64;

    // Do some math to figure out where our start and end are.
    let chain_info = client.get_blockchain_info().await?;
    let start_height = (target - lookback as i64)
        .max(pre_genesis as i64)
        .min(chain_info.blocks as i64) as u64;
    let end_height = chain_info
        .blocks
        .min(pre_genesis.max(target_next_block.saturating_sub(1)));
    debug!(%start_height, %end_height, "queried L1 client, have init range");

    // Loop through the range we've determined to be okay and pull the blocks we want to look back
    // through in.
    let mut real_cur_height = start_height;
    for height in start_height..=end_height {
        let blkid = client.get_block_hash(height).await?;
        debug!(%height, %blkid, "loaded recent L1 block");
        init_queue.push_back(blkid);
        real_cur_height = height;
    }

    let params = ctx.params.clone();
    let mut filter_config = TxFilterConfig::derive_from(params.rollup())?;
    let epoch = ctx.status_channel.get_cur_chain_epoch().unwrap_or(0);

    // update filterconfig based on chainstate of last seen checkpoint
    if let Some(chainstate) = find_last_checkpoint_chainstate(&ctx.storage).await? {
        filter_config.update_from_chainstate(&chainstate);
    }

    let state = ReaderState::new(
        real_cur_height + 1,
        lookback,
        init_queue,
        filter_config,
        epoch,
    );
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
    let client_height = chain_info.blocks;
    let fresh_best_block = chain_info.best_block_hash.parse::<BlockHash>()?;

    if fresh_best_block == *state.best_block() {
        trace!("polled client, nothing to do");
        return Ok(vec![]);
    }

    let mut events = Vec::new();

    // First, check for a reorg if there is one.
    if let Some((pivot_height, pivot_blkid)) = find_pivot_block(ctx.client.as_ref(), state).await? {
        if pivot_height < state.best_block_idx() {
            info!(%pivot_height, %pivot_blkid, "found apparent reorg");
            let block = L1BlockCommitment::new(pivot_height, L1BlockId::from(pivot_blkid));
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
                if let Some(checkpt) = find_checkpoint_in_event(&ev) {
                    // if we have a checkpoint in this block, update filterconfig based on this
                    let chainstate = borsh::from_slice(checkpt.checkpoint().sidecar().chainstate())
                        .expect("deserialize chainstate");

                    state
                        .filter_config_mut()
                        .update_from_chainstate(&chainstate);
                }
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
) -> anyhow::Result<Option<(u64, BlockHash)>> {
    for (height, l1blkid) in state.iter_blocks_back() {
        // If at genesis, we can't reorg any farther.
        if height == 0 {
            return Ok(Some((height, *l1blkid)));
        }

        let queried_l1blkid = client.get_block_hash(height).await?;
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
    height: u64,
    state: &mut ReaderState,
    status_updates: &mut Vec<L1StatusUpdate>,
) -> anyhow::Result<(BlockHash, L1Event)> {
    let block = ctx.client.get_block_at(height).await?;
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
    height: u64,
    block: Block,
) -> anyhow::Result<(L1Event, BlockHash)> {
    let txs = block.txdata.len();

    // Index all the stuff in the block.
    let entries: Vec<RelevantTxEntry> =
        index_block(&block, ReaderTxVisitorImpl::new, state.filter_config());

    // TODO: do stuffs with dep_reqs and da_entries

    let block_data = BlockData::new(height, block, entries);

    let l1blkid = block_data.block().block_hash();

    trace!(%height, %l1blkid, %txs, "fetched block from client");

    status_updates.push(L1StatusUpdate::CurHeight(height));
    status_updates.push(L1StatusUpdate::CurTip(l1blkid.to_string()));

    let block_ev = L1Event::BlockData(block_data, state.epoch());

    Ok((block_ev, l1blkid))
}

/// Retrieves the timestamps for a specified number of blocks starting from the given block height,
/// moving backwards. For each block from `height` down to `height - count + 1`, it fetches the
/// block’s timestamp. If a block height is less than 1 (i.e. there is no block), it inserts a
/// placeholder value of 0. The resulting vector is then reversed so that timestamps are returned in
/// ascending order (oldest first).
async fn fetch_block_timestamps_ascending(
    client: &impl Reader,
    height: u64,
    count: usize,
) -> anyhow::Result<Vec<u32>> {
    let mut timestamps = Vec::with_capacity(count);

    for i in 0..count {
        let current_height = height.saturating_sub(i as u64);
        // If we've gone past block 1, push 0 as a placeholder.
        if current_height < 1 {
            timestamps.push(0);
        } else {
            let header = client.get_block_header_at(current_height).await?;
            timestamps.push(header.time);
        }
    }

    timestamps.reverse();
    Ok(timestamps)
}

pub async fn fetch_genesis_l1_view(
    client: &impl Reader,
    block_height: u64,
) -> anyhow::Result<GenesisL1View> {
    // Create BTC parameters based on the current network.
    let network = client.network().await?;
    let btc_params = BtcParams::from(bitcoin::params::Params::from(network));

    // Get the difficulty adjustment block just before the given block height,
    // representing the start of the current epoch.
    let current_epoch_start_height =
        get_relative_difficulty_adjustment_height(0, block_height, btc_params.inner());
    let current_epoch_start_header = client
        .get_block_header_at(current_epoch_start_height)
        .await?;

    // Fetch the block header at the height
    let block_header = client.get_block_header_at(block_height).await?;

    // Fetch timestamps
    let timestamps =
        fetch_block_timestamps_ascending(client, block_height, TIMESTAMPS_FOR_MEDIAN).await?;
    let timestamps: [u32; TIMESTAMPS_FOR_MEDIAN] = timestamps.try_into().expect(
        "fetch_block_timestamps_ascending should return exactly TIMESTAMPS_FOR_MEDIAN timestamps",
    );

    // Compute the block ID for the verified block.
    let block_id: L1BlockId = block_header.block_hash().into();

    // If (block_height + 1) is the start of the new epoch, we need to calculate the
    // next_block_target, else next_block_target will be current block's target
    let next_block_target =
        if (block_height + 1).is_multiple_of(btc_params.difficulty_adjustment_interval()) {
            CompactTarget::from_next_work_required(
                block_header.bits,
                (block_header.time - current_epoch_start_header.time) as u64,
                &btc_params,
            )
            .to_consensus()
        } else {
            client
                .get_block_header_at(block_height)
                .await?
                .target()
                .to_compact_lossy()
                .to_consensus()
        };

    // Build the genesis L1 view structure.
    let genesis_l1_view = GenesisL1View {
        blk: L1BlockCommitment::new(block_height, block_id),
        next_target: next_block_target,
        epoch_start_timestamp: current_epoch_start_header.time,
        last_11_timestamps: timestamps,
    };

    Ok(genesis_l1_view)
}

/// Returns the [`HeaderVerificationState`] after applying the given block height. This state can be
/// used to verify the next block header.
///
/// This function assumes that `block_height` is valid and gathers all necessary
/// blockchain data, such as difficulty adjustment headers, block timestamps, and target
/// values, to compute the verification state.
///
/// It calculates the current and previous epoch adjustment headers, fetches the required
/// timestamps (including a safe margin for potential reorg depth), and determines the next
/// block's target.
pub async fn fetch_verification_state(
    client: &impl Reader,
    block_height: u64,
) -> anyhow::Result<HeaderVerificationState> {
    // Create BTC parameters based on the current network.
    let network = client.network().await?;
    let genesis_l1_view = fetch_genesis_l1_view(client, block_height).await?;
    // Build the header verification state structure.
    let header_verification_state = HeaderVerificationState::new(network, &genesis_l1_view);

    trace!(%block_height, ?header_verification_state, "HeaderVerificationState");

    Ok(header_verification_state)
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod test {
    use bitcoin::hashes::Hash;
    use strata_primitives::buf::Buf32;
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::test_utils::corepc_node_helpers::{get_bitcoind_and_client, mine_blocks};

    #[tokio::test()]
    async fn test_fetch_timestamps() {
        let (bitcoind, client) = get_bitcoind_and_client();
        let _ = mine_blocks(&bitcoind, 115, None).unwrap();

        let ts = fetch_block_timestamps_ascending(&client, 15, 10)
            .await
            .unwrap();
        assert!(ts.is_sorted());

        let ts = fetch_block_timestamps_ascending(&client, 10, 10)
            .await
            .unwrap();
        assert!(ts.is_sorted());

        let ts = fetch_block_timestamps_ascending(&client, 5, 10)
            .await
            .unwrap();
        assert!(ts.is_sorted());
    }
}
