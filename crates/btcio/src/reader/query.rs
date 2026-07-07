use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::bail;
use bitcoin::{Block, BlockHash, Network};
use bitcoind_async_client::{corepc_types::model::GetBlockchainInfo, traits::Reader};
use strata_btc_types::{BlockHashExt, L1BlockIdBitcoinExt};
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
    rpc_error::{
        is_block_height_out_of_range_error, is_missing_block_height_anyhow_error,
        is_retryable_anyhow_error,
    },
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
        calculate_target_next_block(storage.as_ref(), btcio_params.genesis_l1_height()).await?;

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
async fn calculate_target_next_block(
    storage: &NodeStorage,
    genesis_l1_height: L1Height,
) -> anyhow::Result<L1Height> {
    let stored_l1_target = storage
        .l1()
        .get_canonical_chain_tip_async()
        .await?
        .map(|(height, _)| height.saturating_add(1))
        .unwrap_or(genesis_l1_height);
    let target_next_block = stored_l1_target.max(genesis_l1_height);
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
                Err(err)
                    if is_retryable_anyhow_error(&err)
                        || is_missing_block_height_anyhow_error(&err) =>
                {
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

    let lookback = ctx.btcio_params.l1_reorg_safe_depth() as L1Height * 2;
    let client = ctx.client.as_ref();

    // Do some math to figure out where our start and end are.
    let chain_info = client.get_blockchain_info().await?;
    validate_bitcoind(ctx, &chain_info).await?;

    let stored_canonical_tip = ctx.storage.l1().get_canonical_chain_tip_async().await?;
    let (init_queue, real_cur_height) = match stored_canonical_tip {
        Some((stored_tip_height, _stored_tip_blkid))
            if stored_tip_height.saturating_add(1) >= target_next_block =>
        {
            seed_queue_from_stored_chain(ctx, stored_tip_height, lookback).await?
        }
        stored_tip => {
            let genesis_height = ctx.btcio_params.genesis_l1_height();
            if let Some((stored_tip_height, _)) = stored_tip {
                info!(
                    %stored_tip_height,
                    %target_next_block,
                    %genesis_height,
                    "stored L1 canonical tip is below configured genesis; seeding reader from Bitcoin client"
                );
            }
            seed_queue_from_client(ctx, target_next_block, &chain_info, lookback).await?
        }
    };

    let epoch = ctx.status_channel.get_cur_chain_epoch().unwrap_or(0);

    // Note: Transaction filtering is no longer needed since the ASM STF handles
    // parsing L1 blocks and producing manifests with logs.
    let state = ReaderState::new(real_cur_height + 1, lookback as usize, init_queue, epoch);
    Ok(state)
}

/// Validates Bitcoin client chain properties before reader ingestion starts.
async fn validate_bitcoind<R: Reader>(
    ctx: &ReaderContext<R>,
    chain_info: &GetBlockchainInfo,
) -> anyhow::Result<()> {
    if chain_info.chain != ctx.expected_network {
        bail!(
            "btcio: bitcoind network mismatch: expected {}, got {}",
            ctx.expected_network,
            chain_info.chain
        );
    }

    let actual_hash = ctx
        .client
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

    Ok(())
}

/// Seeds a [`ReaderState`] queue from the stored canonical L1 chain.
async fn seed_queue_from_stored_chain<R: Reader>(
    ctx: &ReaderContext<R>,
    stored_tip_height: L1Height,
    lookback: L1Height,
) -> anyhow::Result<(VecDeque<BlockHash>, L1Height)> {
    let start_height = stored_tip_height.saturating_sub(lookback);
    let genesis_height = ctx.btcio_params.genesis_l1_height();
    let mut init_queue = VecDeque::new();
    let mut height = stored_tip_height;

    loop {
        match ctx
            .storage
            .l1()
            .get_canonical_blockid_at_height_async(height)
            .await?
        {
            Some(blockid) => init_queue.push_front(blockid.to_block_hash()),
            None if height < genesis_height => break,
            None if height == stored_tip_height => {
                bail!(
                    "btcio: stored L1 canonical chain tip at height {stored_tip_height} \
                     was missing from the reader startup walk"
                );
            }
            None => {
                bail!(
                    "btcio: stored L1 canonical chain has a gap at height {height} \
                     while seeding reader state from tip {stored_tip_height}"
                );
            }
        }

        if height == start_height {
            break;
        }

        height = height.saturating_sub(1);
    }

    let loaded_start_height =
        stored_tip_height.saturating_sub(init_queue.len().saturating_sub(1) as L1Height);
    debug!(
        %loaded_start_height,
        end_height = %stored_tip_height,
        entries = init_queue.len(),
        "loaded reader init range from stored L1 canonical chain"
    );

    if init_queue.is_empty() {
        bail!(
            "btcio: stored L1 canonical chain tip at height {stored_tip_height} \
             was missing from the reader startup walk"
        );
    }

    Ok((init_queue, stored_tip_height))
}

/// Seeds a [`ReaderState`] queue from the Bitcoin client.
async fn seed_queue_from_client<R: Reader>(
    ctx: &ReaderContext<R>,
    target_next_block: L1Height,
    chain_info: &GetBlockchainInfo,
    lookback: L1Height,
) -> anyhow::Result<(VecDeque<BlockHash>, L1Height)> {
    let client = ctx.client.as_ref();
    let genesis_height = ctx.btcio_params.genesis_l1_height();
    let pre_genesis = genesis_height.saturating_sub(1);
    let chain_tip = chain_info.blocks as L1Height;
    let start_height = target_next_block
        .saturating_sub(lookback)
        .max(pre_genesis)
        .min(chain_tip);
    let end_height = chain_tip.min(pre_genesis.max(target_next_block.saturating_sub(1)));
    let mut init_queue = VecDeque::new();
    let mut real_cur_height = start_height;

    debug!(%start_height, %end_height, "queried L1 client, have init range");

    // Loop through the range we've determined to be okay and pull the blocks we want to
    // look back through in.
    for height in start_height..=end_height {
        let blkid = client.get_block_hash(height as u64).await?;
        debug!(%height, %blkid, "loaded recent L1 block");
        init_queue.push_back(blkid);
        real_cur_height = height;
    }

    Ok((init_queue, real_cur_height))
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
        let reader_best_height = state.best_block_idx();
        if client_height < reader_best_height && pivot_height == client_height {
            debug!(
                %client_height,
                %reader_best_height,
                %pivot_blkid,
                "Bitcoin client tip is a prefix of reader state; waiting for client to catch up"
            );
            return Ok(vec![]);
        }

        if pivot_height < reader_best_height {
            info!(%pivot_height, %pivot_blkid, "found apparent reorg");
            let block = L1BlockCommitment::new(pivot_height, pivot_blkid.to_l1_block_id());
            state.rollback_to_height(pivot_height);

            // Return with the revert event immediately
            let revert_ev = L1Event::RevertTo(block);
            return Ok(vec![revert_ev]);
        }
    } else {
        let reader_best_height = state.best_block_idx();
        let lowest_tracked_height = state
            .iter_blocks_back()
            .last()
            .map(|(height, _)| height)
            .expect("reader: recent blocks is nonempty");
        if client_height < lowest_tracked_height
            && client_tip_matches_stored_canonical(ctx, client_height, fresh_best_block).await?
        {
            debug!(
                %client_height,
                %reader_best_height,
                %fresh_best_block,
                %lowest_tracked_height,
                "Bitcoin client tip is a deeply lagging prefix of stored reader state; waiting for client to catch up"
            );
            return Ok(vec![]);
        }

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

async fn client_tip_matches_stored_canonical<R: Reader>(
    ctx: &ReaderContext<R>,
    client_height: L1Height,
    client_tip_hash: BlockHash,
) -> anyhow::Result<bool> {
    let Some(stored_blockid) = ctx
        .storage
        .l1()
        .get_canonical_blockid_at_height_async(client_height)
        .await?
    else {
        debug!(
            %client_height,
            %client_tip_hash,
            "stored L1 canonical chain is missing deeply lagging Bitcoin client tip height"
        );
        return Ok(false);
    };

    let client_tip_blockid = client_tip_hash.to_l1_block_id();
    if stored_blockid != client_tip_blockid {
        debug!(
            %client_height,
            %client_tip_hash,
            %stored_blockid,
            "deeply lagging Bitcoin client tip does not match stored L1 canonical chain"
        );
        return Ok(false);
    }

    Ok(true)
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

        let queried_l1blkid = match client.get_block_hash(height as u64).await {
            Ok(block_hash) => block_hash,
            Err(err) if is_block_height_out_of_range_error(&err) => {
                trace!(
                    %height,
                    %l1blkid,
                    err = %err,
                    "Bitcoin client does not have tracked block height while finding pivot"
                );
                continue;
            }
            Err(err) => return Err(err.into()),
        };
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

    use bitcoin::{block::Header, hashes::Hash, Block, BlockHash, Network, Txid};
    use bitcoind_async_client::{
        corepc_types::model::{
            EstimateSmartFee, GetBlockchainInfo, GetMempoolInfo, GetRawMempool,
            GetRawMempoolVerbose, GetRawTransaction, GetRawTransactionVerbose, GetTxOut,
        },
        error::ClientError,
        ClientResult,
    };
    use strata_config::btcio::ReaderConfig;
    use strata_csm_types::{ClientState, ClientUpdateOutput, L1Status};
    use strata_db_store_sled::{test_utils::get_test_sled_backend, SledBackend};
    use strata_db_types::{backend::DatabaseBackend, l1::L1Database};
    use strata_l1_txfmt::MagicBytes;
    use strata_primitives::l1::{L1BlockCommitment, L1BlockId};
    use strata_status::StatusChannel;
    use strata_storage::{create_node_storage, test_runtime_handle, NodeStorage};

    use super::*;
    use crate::test_utils::TestBitcoinClient;

    #[derive(Debug, Clone)]
    struct ChainBitcoinClient {
        blocks: Vec<Block>,
        inner: TestBitcoinClient,
    }

    impl ChainBitcoinClient {
        fn new(blocks: Vec<Block>) -> Self {
            assert!(!blocks.is_empty());
            Self {
                blocks,
                inner: TestBitcoinClient::new(0),
            }
        }

        fn block_hash(&self, height: L1Height) -> BlockHash {
            self.blocks[height as usize].block_hash()
        }

        fn block_by_hash(&self, hash: &BlockHash) -> ClientResult<Block> {
            self.blocks
                .iter()
                .find(|block| block.block_hash() == *hash)
                .cloned()
                .ok_or_else(|| ClientError::Server(-8, format!("block not found: {hash}")))
        }

        fn block_at(&self, height: u64) -> ClientResult<Block> {
            self.blocks.get(height as usize).cloned().ok_or_else(|| {
                ClientError::Server(-8, format!("block height out of range: {height}"))
            })
        }
    }

    impl Reader for ChainBitcoinClient {
        async fn estimate_smart_fee(&self, conf_target: u16) -> ClientResult<EstimateSmartFee> {
            self.inner.estimate_smart_fee(conf_target).await
        }

        async fn get_block_header(&self, hash: &BlockHash) -> ClientResult<Header> {
            Ok(self.block_by_hash(hash)?.header)
        }

        async fn get_block(&self, hash: &BlockHash) -> ClientResult<Block> {
            self.block_by_hash(hash)
        }

        async fn get_block_height(&self, hash: &BlockHash) -> ClientResult<u64> {
            self.blocks
                .iter()
                .position(|block| block.block_hash() == *hash)
                .map(|height| height as u64)
                .ok_or_else(|| ClientError::Server(-8, format!("block not found: {hash}")))
        }

        async fn get_block_header_at(&self, height: u64) -> ClientResult<Header> {
            Ok(self.block_at(height)?.header)
        }

        async fn get_block_at(&self, height: u64) -> ClientResult<Block> {
            self.block_at(height)
        }

        async fn get_block_count(&self) -> ClientResult<u64> {
            Ok(self.blocks.len() as u64 - 1)
        }

        async fn get_block_hash(&self, height: u64) -> ClientResult<BlockHash> {
            Ok(self.block_at(height)?.block_hash())
        }

        async fn get_blockchain_info(&self) -> ClientResult<GetBlockchainInfo> {
            let mut info = self.inner.get_blockchain_info().await?;
            let block_count = self.blocks.len() as u32 - 1;
            info.blocks = block_count;
            info.headers = block_count;
            info.best_block_hash = self
                .blocks
                .last()
                .expect("test: nonempty chain")
                .block_hash();
            Ok(info)
        }

        async fn get_current_timestamp(&self) -> ClientResult<u32> {
            self.inner.get_current_timestamp().await
        }

        async fn get_raw_mempool(&self) -> ClientResult<GetRawMempool> {
            self.inner.get_raw_mempool().await
        }

        async fn get_raw_mempool_verbose(&self) -> ClientResult<GetRawMempoolVerbose> {
            self.inner.get_raw_mempool_verbose().await
        }

        async fn get_mempool_info(&self) -> ClientResult<GetMempoolInfo> {
            self.inner.get_mempool_info().await
        }

        async fn get_raw_transaction_verbosity_zero(
            &self,
            txid: &Txid,
        ) -> ClientResult<GetRawTransaction> {
            self.inner.get_raw_transaction_verbosity_zero(txid).await
        }

        async fn get_raw_transaction_verbosity_one(
            &self,
            txid: &Txid,
        ) -> ClientResult<GetRawTransactionVerbose> {
            self.inner.get_raw_transaction_verbosity_one(txid).await
        }

        async fn get_tx_out(
            &self,
            txid: &Txid,
            vout: u32,
            include_mempool: bool,
        ) -> ClientResult<GetTxOut> {
            self.inner.get_tx_out(txid, vout, include_mempool).await
        }

        async fn network(&self) -> ClientResult<Network> {
            self.inner.network().await
        }
    }

    fn test_storage() -> NodeStorage {
        let (_, storage) = test_storage_with_backend();
        storage
    }

    fn test_storage_with_backend() -> (Arc<SledBackend>, NodeStorage) {
        let backend = get_test_sled_backend();
        let storage = create_node_storage(backend.clone(), test_runtime_handle())
            .expect("test: create node storage");
        (backend, storage)
    }

    fn l1_block(height: L1Height) -> L1BlockCommitment {
        L1BlockCommitment::new(height, L1BlockId::default())
    }

    async fn store_client_state(storage: &NodeStorage, height: L1Height) {
        let block = l1_block(height);
        storage
            .client_state()
            .put_update_async(
                &block,
                ClientUpdateOutput::new_state(ClientState::default()),
            )
            .await
            .expect("test: put client state");
    }

    async fn store_l1_canonical(storage: &NodeStorage, height: L1Height) {
        storage
            .l1()
            .extend_canonical_chain_async(&L1BlockId::default(), height)
            .await
            .expect("test: extend canonical chain");
    }

    async fn store_l1_canonical_hash(
        storage: &NodeStorage,
        height: L1Height,
        block_hash: BlockHash,
    ) {
        storage
            .l1()
            .extend_canonical_chain_async(&block_hash.to_l1_block_id(), height)
            .await
            .expect("test: extend canonical chain");
    }

    fn block_hash(byte: u8) -> BlockHash {
        BlockHash::from_byte_array([byte; 32])
    }

    async fn test_block(nonce: u32) -> Block {
        let mut block = TestBitcoinClient::new(0)
            .get_block_at(0)
            .await
            .expect("test: get base block");
        block.header.nonce = nonce;
        block
    }

    async fn chain_client(nonces: &[u32]) -> ChainBitcoinClient {
        let mut blocks = Vec::with_capacity(nonces.len());
        for nonce in nonces {
            blocks.push(test_block(*nonce).await);
        }
        ChainBitcoinClient::new(blocks)
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

    fn chain_reader_context(
        storage: NodeStorage,
        client: ChainBitcoinClient,
    ) -> ReaderContext<ChainBitcoinClient> {
        chain_reader_context_with_genesis(storage, client, 0)
    }

    fn chain_reader_context_with_genesis(
        storage: NodeStorage,
        client: ChainBitcoinClient,
        genesis_l1_height: L1Height,
    ) -> ReaderContext<ChainBitcoinClient> {
        let expected_l1_anchor = L1BlockCommitment::new(0, client.block_hash(0).to_l1_block_id());
        ReaderContext {
            client: Arc::new(client),
            storage: Arc::new(storage),
            config: Arc::new(ReaderConfig::default()),
            btcio_params: BtcioParams::new(2, MagicBytes::new(*b"ALPN"), genesis_l1_height),
            expected_network: Network::Regtest,
            expected_l1_anchor,
            status_channel: StatusChannel::new(
                ClientState::default(),
                expected_l1_anchor,
                L1Status::default(),
                None,
                None,
            ),
        }
    }

    #[tokio::test]
    async fn calculate_target_next_block_starts_at_genesis_without_stored_l1() {
        let storage = test_storage();

        let target = calculate_target_next_block(&storage, 42)
            .await
            .expect("test: target block");

        assert_eq!(target, 42);
    }

    #[tokio::test]
    async fn calculate_target_next_block_uses_stored_l1_tip() {
        let storage = test_storage();
        store_l1_canonical(&storage, 100).await;

        let target = calculate_target_next_block(&storage, 42)
            .await
            .expect("test: target block");

        assert_eq!(target, 101);
    }

    #[tokio::test]
    async fn calculate_target_next_block_ignores_client_state_without_stored_l1() {
        let storage = test_storage();
        store_client_state(&storage, 100).await;

        let target = calculate_target_next_block(&storage, 42)
            .await
            .expect("test: target block");

        assert_eq!(target, 42);
    }

    #[tokio::test]
    async fn calculate_target_next_block_clamps_pregenesis_l1_tip_to_genesis() {
        let storage = test_storage();
        store_l1_canonical(&storage, 10).await;

        let target = calculate_target_next_block(&storage, 42)
            .await
            .expect("test: target block");

        assert_eq!(target, 42);
    }

    #[tokio::test]
    async fn calculate_target_next_block_ignores_client_state_when_l1_tip_exists() {
        let storage = test_storage();
        store_client_state(&storage, 100).await;
        store_l1_canonical(&storage, 111).await;

        let target = calculate_target_next_block(&storage, 42)
            .await
            .expect("test: target block");

        assert_eq!(target, 112);
    }

    #[tokio::test]
    async fn init_reader_state_seeds_from_stored_canonical_entries() {
        let storage = test_storage();
        let stored_chain = chain_client(&[0, 101, 102, 103, 104, 105, 106]).await;
        let bitcoind_chain = chain_client(&[0, 201, 202, 203, 204, 205, 206]).await;

        for height in 0..=6 {
            store_l1_canonical_hash(&storage, height, stored_chain.block_hash(height)).await;
        }

        let ctx = chain_reader_context(storage, bitcoind_chain);
        let state = init_reader_state(&ctx, 7)
            .await
            .expect("test: initialize reader state");

        assert_eq!(state.next_height(), 7);
        assert_eq!(*state.best_block(), stored_chain.block_hash(6));

        let recent_blocks = state
            .iter_blocks_back()
            .map(|(height, block_hash)| (height, *block_hash))
            .collect::<Vec<_>>();
        let expected_blocks = (2..=6)
            .rev()
            .map(|height| (height, stored_chain.block_hash(height)))
            .collect::<Vec<_>>();
        assert_eq!(recent_blocks, expected_blocks);
    }

    #[tokio::test]
    async fn init_reader_state_errors_on_stored_canonical_gap() {
        let (backend, storage) = test_storage_with_backend();
        let stored_chain = chain_client(&[0, 101, 102, 103, 104, 105, 106]).await;
        let bitcoind_chain = chain_client(&[0, 201, 202, 203, 204, 205, 206]).await;

        for height in 0..=6 {
            store_l1_canonical_hash(&storage, height, stored_chain.block_hash(height)).await;
        }
        backend
            .l1_db()
            .remove_canonical_chain_entries(4, 4)
            .expect("test: remove canonical chain entry");

        let ctx = chain_reader_context(storage, bitcoind_chain);
        let err = init_reader_state(&ctx, 7)
            .await
            .expect_err("test: stored canonical gap should fail initialization");

        assert!(
            err.to_string().contains("gap at height 4"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn init_reader_state_accepts_stored_chain_starting_at_genesis() {
        let storage = test_storage();
        let stored_chain = chain_client(&[0, 101, 102, 103, 104]).await;
        let bitcoind_chain = chain_client(&[0, 201, 202, 203, 204, 205, 206]).await;
        let genesis_height = 2;

        for height in genesis_height..=4 {
            store_l1_canonical_hash(&storage, height, stored_chain.block_hash(height)).await;
        }

        let ctx = chain_reader_context_with_genesis(storage, bitcoind_chain, genesis_height);
        let state = init_reader_state(&ctx, 5)
            .await
            .expect("test: initialize reader state");

        assert_eq!(state.next_height(), 5);
        assert_eq!(*state.best_block(), stored_chain.block_hash(4));

        let recent_blocks = state
            .iter_blocks_back()
            .map(|(height, block_hash)| (height, *block_hash))
            .collect::<Vec<_>>();
        let expected_blocks = (genesis_height..=4)
            .rev()
            .map(|height| (height, stored_chain.block_hash(height)))
            .collect::<Vec<_>>();
        assert_eq!(recent_blocks, expected_blocks);
    }

    #[tokio::test]
    async fn init_reader_state_ignores_pregenesis_stored_tip() {
        let storage = test_storage();
        let stored_chain = chain_client(&[0, 101, 102]).await;
        let bitcoind_chain = chain_client(&[0, 201, 202, 203, 204, 205, 206]).await;
        let genesis_height = 4;

        for height in 0..=2 {
            store_l1_canonical_hash(&storage, height, stored_chain.block_hash(height)).await;
        }

        let ctx =
            chain_reader_context_with_genesis(storage, bitcoind_chain.clone(), genesis_height);
        let state = init_reader_state(&ctx, genesis_height)
            .await
            .expect("test: initialize reader state");

        assert_eq!(state.next_height(), genesis_height);
        assert_eq!(
            *state.best_block(),
            bitcoind_chain.block_hash(genesis_height - 1)
        );

        let recent_blocks = state
            .iter_blocks_back()
            .map(|(height, block_hash)| (height, *block_hash))
            .collect::<Vec<_>>();
        let expected_blocks = Vec::from([(
            genesis_height - 1,
            bitcoind_chain.block_hash(genesis_height - 1),
        )]);
        assert_eq!(recent_blocks, expected_blocks);
    }

    #[tokio::test]
    async fn poll_for_new_blocks_reverts_offline_reorg_then_continues() {
        let storage = test_storage();
        let stored_chain = chain_client(&[0, 1, 2, 103, 104]).await;
        let bitcoind_chain = chain_client(&[0, 1, 2, 203, 204, 205]).await;

        for height in 0..=4 {
            store_l1_canonical_hash(&storage, height, stored_chain.block_hash(height)).await;
        }

        let ctx = chain_reader_context(storage, bitcoind_chain.clone());
        let mut state = init_reader_state(&ctx, 5)
            .await
            .expect("test: initialize reader state");
        let mut status_updates = Vec::new();

        let events = poll_for_new_blocks(&ctx, &mut state, &mut status_updates)
            .await
            .expect("test: poll reorg");

        assert_eq!(events.len(), 1);
        match &events[0] {
            L1Event::RevertTo(block) => {
                assert_eq!(block.height(), 2);
                assert_eq!(
                    *block.blkid(),
                    bitcoind_chain.block_hash(2).to_l1_block_id()
                );
            }
            event => panic!("test: expected RevertTo event, got {event:?}"),
        }
        assert_eq!(state.next_height(), 3);
        assert_eq!(*state.best_block(), bitcoind_chain.block_hash(2));

        ctx.storage
            .l1()
            .revert_canonical_chain_async(2)
            .await
            .expect("test: apply revert");

        let events = poll_for_new_blocks(&ctx, &mut state, &mut status_updates)
            .await
            .expect("test: poll new fork");

        let new_block_heights = events
            .iter()
            .map(|event| match event {
                L1Event::BlockData(block_data, _) => block_data.block_num(),
                L1Event::RevertTo(block) => {
                    panic!(
                        "test: unexpected RevertTo event at height {}",
                        block.height()
                    )
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(new_block_heights, vec![3, 4, 5]);
        assert_eq!(state.next_height(), 6);
        assert_eq!(*state.best_block(), bitcoind_chain.block_hash(5));
    }

    #[tokio::test]
    async fn poll_for_new_blocks_waits_when_bitcoind_tip_is_matching_prefix() {
        let storage = test_storage();
        let stored_chain = chain_client(&[0, 1, 2, 3, 4, 5]).await;
        let bitcoind_chain = chain_client(&[0, 1, 2, 3]).await;

        for height in 0..=5 {
            store_l1_canonical_hash(&storage, height, stored_chain.block_hash(height)).await;
        }

        let ctx = chain_reader_context(storage, bitcoind_chain);
        let mut state = init_reader_state(&ctx, 6)
            .await
            .expect("test: initialize reader state");
        let mut status_updates = Vec::new();

        let events = poll_for_new_blocks(&ctx, &mut state, &mut status_updates)
            .await
            .expect("test: poll matching prefix");

        assert!(events.is_empty());
        assert_eq!(state.next_height(), 6);
        assert_eq!(*state.best_block(), stored_chain.block_hash(5));
    }

    #[tokio::test]
    async fn poll_for_new_blocks_waits_when_deeply_lagging_bitcoind_tip_is_matching_prefix() {
        let storage = test_storage();
        let stored_chain = chain_client(&[0, 1, 2, 3, 104, 105, 106, 107, 108]).await;
        let bitcoind_chain = chain_client(&[0, 1, 2, 3]).await;

        for height in 0..=8 {
            store_l1_canonical_hash(&storage, height, stored_chain.block_hash(height)).await;
        }

        let ctx = chain_reader_context(storage, bitcoind_chain);
        let mut state = init_reader_state(&ctx, 9)
            .await
            .expect("test: initialize reader state");
        let mut status_updates = Vec::new();

        let events = poll_for_new_blocks(&ctx, &mut state, &mut status_updates)
            .await
            .expect("test: poll deeply lagging matching prefix");

        assert!(events.is_empty());
        assert_eq!(state.next_height(), 9);
        assert_eq!(*state.best_block(), stored_chain.block_hash(8));
    }

    #[tokio::test]
    async fn poll_for_new_blocks_errors_when_deeply_lagging_bitcoind_chain_diverges() {
        let storage = test_storage();
        let stored_chain = chain_client(&[0, 1, 2, 3, 104, 105, 106, 107, 108]).await;
        let bitcoind_chain = chain_client(&[0, 1, 202, 203]).await;

        for height in 0..=8 {
            store_l1_canonical_hash(&storage, height, stored_chain.block_hash(height)).await;
        }

        let ctx = chain_reader_context(storage, bitcoind_chain);
        let mut state = init_reader_state(&ctx, 9)
            .await
            .expect("test: initialize reader state");
        let mut status_updates = Vec::new();

        let err = poll_for_new_blocks(&ctx, &mut state, &mut status_updates)
            .await
            .expect_err("test: poll deeply lagging divergent chain");

        assert_eq!(
            err.downcast_ref::<ReaderError>(),
            Some(&ReaderError::PivotNotFound {
                client_height: 3,
                reader_best_height: 8,
                known_depth: 5,
            })
        );
        assert_eq!(state.next_height(), 9);
        assert_eq!(*state.best_block(), stored_chain.block_hash(8));
    }

    #[tokio::test]
    async fn poll_for_new_blocks_reverts_when_shorter_bitcoind_chain_diverges() {
        let storage = test_storage();
        let stored_chain = chain_client(&[0, 1, 2, 103, 104, 105]).await;
        let bitcoind_chain = chain_client(&[0, 1, 202, 203]).await;

        for height in 0..=5 {
            store_l1_canonical_hash(&storage, height, stored_chain.block_hash(height)).await;
        }

        let ctx = chain_reader_context(storage, bitcoind_chain.clone());
        let mut state = init_reader_state(&ctx, 6)
            .await
            .expect("test: initialize reader state");
        let mut status_updates = Vec::new();

        let events = poll_for_new_blocks(&ctx, &mut state, &mut status_updates)
            .await
            .expect("test: poll shorter divergent chain");

        assert_eq!(events.len(), 1);
        match &events[0] {
            L1Event::RevertTo(block) => {
                assert_eq!(block.height(), 1);
                assert_eq!(
                    *block.blkid(),
                    bitcoind_chain.block_hash(1).to_l1_block_id()
                );
            }
            event => panic!("test: expected RevertTo event, got {event:?}"),
        }
        assert_eq!(state.next_height(), 2);
        assert_eq!(*state.best_block(), bitcoind_chain.block_hash(1));
    }

    #[tokio::test]
    async fn init_reader_state_without_stored_l1_seeds_from_bitcoind() {
        let storage = test_storage();
        let bitcoind_chain = chain_client(&[0, 1, 2, 3, 4, 5]).await;
        let ctx = chain_reader_context(storage, bitcoind_chain.clone());

        let state = init_reader_state(&ctx, 3)
            .await
            .expect("test: initialize reader state");

        assert_eq!(state.next_height(), 3);
        let recent_blocks = state
            .iter_blocks_back()
            .map(|(height, block_hash)| (height, *block_hash))
            .collect::<Vec<_>>();
        let expected_blocks = (0..=2)
            .rev()
            .map(|height| (height, bitcoind_chain.block_hash(height)))
            .collect::<Vec<_>>();
        assert_eq!(recent_blocks, expected_blocks);
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
