use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::bail;
use bitcoin::{Block, BlockHash, Network};
use bitcoind_async_client::traits::Reader;
use strata_btc_types::BlockHashExt;
use strata_config::btcio::ReaderConfig;
use strata_primitives::l1::{L1BlockCommitment, L1Height};
use strata_state::BlockSubmitter;
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use thiserror::Error;
use tokio::time::sleep;
use tracing::*;

use super::event::L1Event;
use crate::{
    reader::{event::BlockData, handler::handle_bitcoin_event, state::ReaderState},
    rpc_error::is_retryable_anyhow_error,
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

    /// Expected Bitcoin network.
    pub expected_network: Network,

    /// L1 anchor block to verify before ingesting reader data.
    pub expected_l1_anchor: L1BlockCommitment,

    /// Status transmitter
    pub status_channel: StatusChannel,
}

/// Expected Bitcoin chain properties validated before reader ingestion starts.
#[derive(Debug, Clone)]
pub struct ReaderValidation {
    expected_network: Network,
    expected_l1_anchor: L1BlockCommitment,
}

impl ReaderValidation {
    /// Creates a validation config for reader startup.
    pub fn new(expected_network: Network, expected_l1_anchor: L1BlockCommitment) -> Self {
        Self {
            expected_network,
            expected_l1_anchor,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
enum ReaderError {
    #[error(
        "btcio: unable to find common L1 block with Bitcoin client chain \
         (client height {client_height}, reader best height {reader_best_height}, \
         known depth {known_depth})"
    )]
    PivotNotFound {
        client_height: L1Height,
        reader_best_height: L1Height,
        known_depth: usize,
    },
}

/// The main task that initializes the reader state and starts reading from bitcoin.
pub async fn bitcoin_data_reader_task<E: BlockSubmitter>(
    client: Arc<impl Reader>,
    storage: Arc<NodeStorage>,
    config: Arc<ReaderConfig>,
    btcio_params: BtcioParams,
    validation: ReaderValidation,
    status_channel: StatusChannel,
    event_submitter: Arc<E>,
) -> anyhow::Result<()> {
    let target_next_block =
        calculate_target_next_block(storage.as_ref(), btcio_params.genesis_l1_height())?;

    let ctx = ReaderContext {
        client,
        storage,
        config,
        btcio_params,
        expected_network: validation.expected_network,
        expected_l1_anchor: validation.expected_l1_anchor,
        status_channel,
    };
    do_reader_task(ctx, target_next_block, event_submitter.as_ref()).await
}

/// Calculates target next block to start polling l1 from.
fn calculate_target_next_block(
    storage: &NodeStorage,
    genesis_l1_height: L1Height,
) -> anyhow::Result<L1Height> {
    let target_next_block = storage
        .client_state()
        .fetch_most_recent_state()?
        .map(|(block, _)| block.height().saturating_add(1))
        .unwrap_or(genesis_l1_height)
        .max(genesis_l1_height);
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
    let mut state: Option<ReaderState> = None;

    loop {
        let mut status_updates: Vec<L1StatusUpdate> = Vec::new();

        if let Some(reader_state) = state.as_mut() {
            match poll_for_new_blocks(&ctx, reader_state, &mut status_updates).await {
                Err(err) => {
                    handle_poll_error(&err, &mut status_updates);
                }
                Ok(events) => {
                    // handle events
                    for ev in events {
                        handle_bitcoin_event(ev, &ctx, event_submitter).await?;
                    }
                }
            }
        } else {
            match init_reader_state(&ctx, target_next_block).await {
                Ok(reader_state) => {
                    let best_blkid = reader_state.best_block();
                    info!(%best_blkid, "initialized L1 reader state");
                    status_updates.push(L1StatusUpdate::RpcConnected(true));
                    state = Some(reader_state);
                }
                Err(err) if is_retryable_anyhow_error(&err) => {
                    handle_poll_error(&err, &mut status_updates);
                }
                Err(err) => return Err(err),
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

    if is_retryable_anyhow_error(err) {
        status_updates.push(L1StatusUpdate::RpcConnected(false));
        return;
    }

    if let Some(reqwest_err) = err.downcast_ref::<reqwest::Error>() {
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
    if chain_info.chain != ctx.expected_network {
        bail!(
            "btcio: bitcoind network mismatch: expected {}, got {}",
            ctx.expected_network,
            chain_info.chain
        );
    }

    let actual_hash = client
        .get_block_hash(ctx.expected_l1_anchor.height() as u64)
        .await?;
    let actual_block_id = actual_hash.to_l1_block_id();
    if actual_block_id != *ctx.expected_l1_anchor.blkid() {
        bail!(
            "btcio: L1 anchor block hash mismatch at height {height}: expected {expected}, got {actual_block_id}",
            height = ctx.expected_l1_anchor.height(),
            expected = ctx.expected_l1_anchor.blkid(),
        );
    }

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
        let reader_best_height = state.best_block_idx();
        let known_depth = state.iter_blocks_back().count();
        let err = ReaderError::PivotNotFound {
            client_height,
            reader_best_height,
            known_depth,
        };
        error!(
            client_height,
            reader_best_height, known_depth, "unable to find common block with client chain"
        );
        return Err(err.into());
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

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Arc};

    use bitcoin::{hashes::Hash, BlockHash, Network};
    use strata_config::btcio::ReaderConfig;
    use strata_csm_types::{ClientState, ClientUpdateOutput, L1Status};
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_l1_txfmt::MagicBytes;
    use strata_primitives::l1::{L1BlockCommitment, L1BlockId};
    use strata_status::StatusChannel;
    use strata_storage::{create_node_storage, NodeStorage};
    use threadpool::ThreadPool;

    use super::*;
    use crate::test_utils::TestBitcoinClient;

    fn test_storage() -> NodeStorage {
        create_node_storage(get_test_sled_backend(), ThreadPool::new(1))
            .expect("test: create node storage")
    }

    fn l1_block(height: L1Height) -> L1BlockCommitment {
        L1BlockCommitment::new(height, L1BlockId::default())
    }

    fn store_client_state(storage: &NodeStorage, height: L1Height) {
        let block = l1_block(height);
        storage
            .client_state()
            .put_update_blocking(
                &block,
                ClientUpdateOutput::new_state(ClientState::default()),
            )
            .expect("test: put client state");
    }

    fn block_hash(byte: u8) -> BlockHash {
        BlockHash::from_byte_array([byte; 32])
    }

    fn reader_context(storage: NodeStorage) -> ReaderContext<TestBitcoinClient> {
        ReaderContext {
            client: Arc::new(TestBitcoinClient::new(0)),
            storage: Arc::new(storage),
            config: Arc::new(ReaderConfig::default()),
            btcio_params: BtcioParams::new(2, MagicBytes::new(*b"ALPN"), 0),
            expected_network: Network::Regtest,
            expected_l1_anchor: l1_block(0),
            status_channel: StatusChannel::new(
                ClientState::default(),
                l1_block(0),
                L1Status::default(),
                None,
                None,
            ),
        }
    }

    #[test]
    fn calculate_target_next_block_falls_back_to_genesis_without_client_state() {
        let storage = test_storage();

        let target = calculate_target_next_block(&storage, 42).expect("test: target block");

        assert_eq!(target, 42);
    }

    #[test]
    fn calculate_target_next_block_uses_latest_client_state_height() {
        let storage = test_storage();
        store_client_state(&storage, 100);

        let target = calculate_target_next_block(&storage, 42).expect("test: target block");

        assert_eq!(target, 101);
    }

    #[test]
    fn calculate_target_next_block_clamps_pregenesis_client_state_to_genesis() {
        let storage = test_storage();
        store_client_state(&storage, 10);

        let target = calculate_target_next_block(&storage, 42).expect("test: target block");

        assert_eq!(target, 42);
    }

    #[tokio::test]
    async fn poll_for_new_blocks_reports_structured_pivot_failure() {
        let ctx = reader_context(test_storage());
        let mut state = ReaderState::new(3, 2, VecDeque::from([block_hash(1), block_hash(2)]), 0);
        let mut status_updates = Vec::new();

        let err = poll_for_new_blocks(&ctx, &mut state, &mut status_updates)
            .await
            .expect_err("test: pivot failure");

        assert_eq!(
            err.downcast_ref::<ReaderError>(),
            Some(&ReaderError::PivotNotFound {
                client_height: 100,
                reader_best_height: 2,
                known_depth: 2,
            })
        );
    }
}
