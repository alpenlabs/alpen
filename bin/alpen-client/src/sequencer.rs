//! Sequencer subsystem setup helpers.
//!
//! Each function encapsulates one concern of the sequencer startup:
//! chunk witness pipeline, batch builder, DA pipeline, and prover setup.

use std::sync::Arc;

use alpen_ee_common::{
    require_latest_batch, BatchStorage, BlockNumHash, ChunkWitnessExtractFn, ChunkWitnessRecord,
    DaBlobSource,
};
use alpen_ee_da_provider::{ChunkedEnvelopeDaProvider, StateDiffBlobProvider};
use alpen_ee_database::{EeDatabases, EeNodeStorage};
use alpen_ee_exec_chain::ExecChainHandle;
use alpen_ee_sequencer::{
    backfill_missing_chunk_witnesses, chunk_witness_channel, chunk_witness_task,
    create_batch_builder, init_batch_builder_state, BatchBuilderHandle, BlockCountDataProvider,
    ChunkExtractRequest, ComposedDataProvider, FixedBlockCountSealing, MaxGasSealing, OrSealing,
};
use alpen_reth_witness::RangeWitnessExtractor;
use bitcoind_async_client::{
    corepc_types::bitcoin::key::Keypair, traits::Wallet as _, Auth, Client as BtcClient,
};
use eyre::Context;
use reth_primitives::Block;
use reth_provider::{BlockReader, HeaderProvider, StateProviderFactory};
use strata_bridge_params::BridgeParams;
use strata_btcio::{
    broadcaster::BroadcasterBuilder, writer::chunked_envelope::create_chunked_envelope_task,
    BtcioParams,
};
use strata_config::btcio::WriterConfig;
use strata_paas::{ProverBuilder, ProverServiceBuilder, ReceiptStore, RetryConfig, TaskStore};
use strata_proofimpl_alpen_acct::EeAcctProgram;
use strata_proofimpl_alpen_chunk::EeChunkProgram;
use strata_service::AsyncExecutor;
#[cfg(feature = "sp1")]
use strata_zkvm_hosts::sp1::{alpen_acct_host, alpen_chunk_host};
use tokio::sync::{mpsc, watch};
use tracing::{error, info, Span};
#[cfg(feature = "sp1")]
use zkaleido_sp1_host::{SP1Host, SP1HostConfig};

use crate::{
    gas_data_provider::RethGasDataProvider,
    genesis,
    header_summary::RethHeaderSummaryProvider,
    prover::{
        AcctRangeWitnessFn, AcctReceiptHook, AcctSpec, ChunkReceiptHook, ChunkSpec,
        EeBatchProofDbManager, EeChunkReceiptStore, EeProverTaskDbManager, PaasBatchProver,
    },
    AdditionalConfig,
};

// ---------------------------------------------------------------------------
// Chunk witness pipeline
// ---------------------------------------------------------------------------

/// Builds the chunk witness extraction pipeline.
///
/// Returns the sender for chunk extraction requests, the main witness task
/// future, and a backfill task that recovers any chunks sealed without a
/// witness row.
pub(crate) fn build_chunk_witness_pipeline<F>(
    range_witness_extractor: Arc<RangeWitnessExtractor<F, EeNodeStorage>>,
    storage: Arc<EeNodeStorage>,
    tasks_span: Span,
) -> (
    mpsc::Sender<ChunkExtractRequest>,
    impl std::future::Future<Output = ()>,
    impl std::future::Future<Output = ()>,
)
where
    F: StateProviderFactory + BlockReader<Block = Block> + Clone + Send + Sync + 'static,
{
    let chunk_witness_extract_fn: Arc<ChunkWitnessExtractFn> = {
        let extractor = range_witness_extractor;
        Arc::new(move |first_block, last_block| {
            let first_b256 = alloy_primitives::B256::from(first_block.0);
            let last_b256 = alloy_primitives::B256::from(last_block.0);
            let data = extractor.extract_range_witness(first_b256, last_b256)?;
            let prev_header_rlp = alloy_rlp::encode(&data.prev_header);
            let blocks_rlp: Vec<Vec<u8>> = data.blocks.iter().map(alloy_rlp::encode).collect();
            Ok(ChunkWitnessRecord::new(
                data.raw_partial_pre_state,
                prev_header_rlp,
                blocks_rlp,
            ))
        })
    };

    let (chunk_witness_tx, chunk_witness_rx) = chunk_witness_channel();
    let chunk_witness_store: Arc<dyn alpen_ee_common::ChunkWitnessStore> = storage.clone();
    let chunk_witness_task_fut = chunk_witness_task(
        chunk_witness_extract_fn,
        chunk_witness_store,
        chunk_witness_rx,
    );

    let chunk_witness_backfill_task = {
        let batch_storage: Arc<dyn BatchStorage> = storage.clone();
        let witness_store: Arc<dyn alpen_ee_common::ChunkWitnessStore> = storage;
        let tx = chunk_witness_tx.clone();
        async move {
            if let Err(e) = backfill_missing_chunk_witnesses(
                batch_storage.as_ref(),
                witness_store.as_ref(),
                &tx,
            )
            .await
            {
                error!(parent: &tasks_span, error = %e, "chunk witness backfill failed at startup");
            }
        }
    };

    (
        chunk_witness_tx,
        chunk_witness_task_fut,
        chunk_witness_backfill_task,
    )
}

// ---------------------------------------------------------------------------
// Batch builder
// ---------------------------------------------------------------------------

/// Initialises batch builder state, validates config, and creates the batch
/// builder task.
pub(crate) async fn build_batch_builder<F>(
    storage: Arc<EeNodeStorage>,
    provider: F,
    genesis_info: &genesis::BlockInfo,
    preconf_rx: watch::Receiver<BlockNumHash>,
    exec_chain_handle: ExecChainHandle,
    chunk_witness_tx: mpsc::Sender<ChunkExtractRequest>,
    ext: &AdditionalConfig,
) -> eyre::Result<(BatchBuilderHandle, impl std::future::Future<Output = ()>)>
where
    F: HeaderProvider + Send + Sync + 'static,
{
    let batch_builder_state = init_batch_builder_state(storage.as_ref())
        .await
        .context("batch builder state initialization should not fail")?;

    let (latest_batch, _) = require_latest_batch(storage.as_ref()).await?;

    if let Some(configured) = ext.batch_sealing_gas_limit {
        let min_batch_gas = ext.custom_chain.genesis().gas_limit.saturating_mul(2);
        eyre::ensure!(
            configured >= min_batch_gas,
            "--batch-sealing-gas-limit ({configured}) is below the minimum \
             ({min_batch_gas}, 2× genesis block gas limit {}). A single block \
             can use up to the per-block gas limit, so the batch budget must \
             be large enough to always fit at least one block.",
            ext.custom_chain.genesis().gas_limit,
        );
    }

    let batch_gas_limit = ext.batch_sealing_gas_limit.unwrap_or(u64::MAX);
    let batch_sealing_policy = OrSealing::new(
        FixedBlockCountSealing::new(ext.batch_sealing_block_count),
        MaxGasSealing::new(batch_gas_limit),
    );
    let block_data_provider = Arc::new(ComposedDataProvider::new(
        BlockCountDataProvider,
        RethGasDataProvider::new(provider),
    ));

    Ok(create_batch_builder(
        latest_batch.id(),
        BlockNumHash::new(genesis_info.blockhash().0.into(), genesis_info.blocknum()),
        batch_builder_state,
        preconf_rx,
        block_data_provider,
        batch_sealing_policy,
        storage.clone(),
        storage.clone(),
        exec_chain_handle,
        Some(chunk_witness_tx),
    ))
}

// ---------------------------------------------------------------------------
// DA pipeline
// ---------------------------------------------------------------------------

/// Handles returned by [`start_da_pipeline`].
pub(crate) struct DaPipelineHandles {
    pub batch_da_provider: Arc<ChunkedEnvelopeDaProvider>,
    pub blob_provider: Arc<dyn DaBlobSource>,
    pub btc_client: Arc<BtcClient>,
    pub da_context_db: Arc<alpen_reth_db::sled::EeDaContextDb<alpen_reth_db::sled::WitnessDB>>,
    pub envelope_watcher_task: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
}

/// Creates the Bitcoin DA pipeline: BTC RPC client, broadcaster service,
/// chunked envelope writer, blob provider, and the DA provider that ties
/// them together.
#[expect(
    clippy::too_many_arguments,
    reason = "sequencer init wiring; will consolidate into context struct"
)]
pub(crate) async fn start_da_pipeline<F>(
    ext: &AdditionalConfig,
    writer_config: Arc<WriterConfig>,
    sequencer_keypair: Keypair,
    dbs: &EeDatabases,
    db_pool: threadpool::ThreadPool,
    storage: Arc<EeNodeStorage>,
    provider: F,
    service_executor: &impl AsyncExecutor,
    tasks_span: &Span,
) -> eyre::Result<DaPipelineHandles>
where
    F: StateProviderFactory
        + BlockReader<Block = Block>
        + HeaderProvider<Header = reth_primitives::Header>
        + Clone
        + Send
        + Sync
        + 'static,
{
    let magic_bytes = ext.ee_da_magic_bytes.expect("enforced by clap");
    let btc_url = ext.btc_rpc_url.as_ref().expect("enforced by clap");
    let btc_user = ext.btc_rpc_user.as_ref().expect("enforced by clap");
    let btc_pass = ext.btc_rpc_password.as_ref().expect("enforced by clap");

    let btcio_params =
        BtcioParams::new(ext.l1_reorg_safe_depth, magic_bytes, ext.genesis_l1_height);

    let btc_client = Arc::new(
        BtcClient::new(
            btc_url.clone(),
            Auth::UserPass(btc_user.clone(), btc_pass.clone()),
            Some(ext.btcio_retry_count),
            Some(ext.btcio_retry_interval),
            None,
        )
        .map_err(|e| eyre::eyre!("creating Bitcoin RPC client: {e}"))?,
    );
    info!(
        parent: tasks_span,
        retry_count = ext.btcio_retry_count,
        retry_interval_ms = ext.btcio_retry_interval,
        "btcio Bitcoin RPC retry policy configured",
    );

    let sequencer_address = btc_client
        .get_new_address()
        .await
        .map_err(|e| eyre::eyre!("failed to get sequencer address: {e}"))?;

    let broadcast_ops = Arc::new(dbs.broadcast_ops(db_pool.clone()));
    let envelope_ops = Arc::new(dbs.chunked_envelope_ops(db_pool));

    let broadcast_handle = Arc::new(
        BroadcasterBuilder::new(btc_client.clone(), broadcast_ops.clone(), btcio_params)
            .with_broadcast_poll_interval_ms(5_000)
            .launch(service_executor)
            .await
            .map_err(|e| eyre::eyre!("starting broadcaster service: {e}"))?,
    );

    let (envelope_handle, envelope_watcher_task) = create_chunked_envelope_task(
        btc_client.clone(),
        writer_config,
        btcio_params,
        sequencer_address,
        sequencer_keypair,
        envelope_ops,
        broadcast_handle.clone(),
    )
    .map_err(|e| eyre::eyre!("creating chunked envelope task: {e}"))?;

    let header_summary = Arc::new(RethHeaderSummaryProvider::new(provider));

    let da_context_db = dbs.da_context_db();
    let blob_provider: Arc<dyn DaBlobSource> = Arc::new(StateDiffBlobProvider::new(
        storage,
        dbs.witness_db(),
        header_summary,
        da_context_db.clone(),
    ));

    let batch_da_provider = Arc::new(ChunkedEnvelopeDaProvider::new(
        blob_provider.clone(),
        envelope_handle,
        broadcast_ops,
        btc_client.clone(),
        magic_bytes,
    )?);

    info!(parent: tasks_span, "btcio DA pipeline started");

    Ok(DaPipelineHandles {
        batch_da_provider,
        blob_provider,
        btc_client,
        da_context_db,
        envelope_watcher_task: Box::pin(envelope_watcher_task),
    })
}

// ---------------------------------------------------------------------------
// Prover setup
// ---------------------------------------------------------------------------

/// Builds and launches the chunk + account prover services, returning the
/// composite batch prover handle.
#[expect(
    clippy::too_many_arguments,
    reason = "sequencer init wiring; will consolidate into context struct"
)]
pub(crate) async fn start_provers<F>(
    ext: &AdditionalConfig,
    dbs: &EeDatabases,
    storage: Arc<EeNodeStorage>,
    btc_client: Arc<BtcClient>,
    range_witness_extractor: Arc<RangeWitnessExtractor<F, EeNodeStorage>>,
    resolved_max_withdrawal: Option<u64>,
    service_executor: &impl AsyncExecutor,
    tasks_span: &Span,
) -> eyre::Result<Arc<PaasBatchProver>>
where
    F: StateProviderFactory + BlockReader<Block = Block> + Clone + Send + Sync + 'static,
{
    let prover_db = dbs.prover_db();
    let task_store: Arc<dyn TaskStore> = Arc::new(EeProverTaskDbManager::new(prover_db.clone()));
    let chunk_receipts: Arc<dyn ReceiptStore> =
        Arc::new(EeChunkReceiptStore::new(prover_db.clone()));
    let batch_proofs = Arc::new(EeBatchProofDbManager::new(prover_db));
    let batch_storage_dyn: Arc<dyn BatchStorage> = storage.clone();

    let genesis = {
        use alpen_reth_exex::alloy2reth::IntoRspChainConfig as _;
        ext.custom_chain.genesis().config.clone().into_rsp()
    };

    let bridge_params = BridgeParams::new(ext.bridge_denomination, resolved_max_withdrawal)
        .expect("invalid withdrawal params");

    let chunk_builder = ProverBuilder::new(ChunkSpec::new(
        batch_storage_dyn.clone(),
        storage.clone(),
        genesis.clone(),
        bridge_params,
    ))
    .task_store(task_store.clone())
    .receipt_store(chunk_receipts.clone())
    .receipt_hook(ChunkReceiptHook::new(batch_storage_dyn.clone()))
    .retry(RetryConfig::default());

    let acct_range_witness_fn: Arc<AcctRangeWitnessFn> = {
        let extractor = range_witness_extractor;
        Arc::new(move |first_block, last_block| {
            extractor.extract_range_witness(first_block, last_block)
        })
    };

    let acct_builder = ProverBuilder::new(AcctSpec::new(
        chunk_receipts.clone(),
        batch_storage_dyn.clone(),
        storage,
        btc_client,
        dbs.witness_db(),
        acct_range_witness_fn,
        genesis,
        bridge_params,
    ))
    .task_store(task_store)
    .receipt_hook(AcctReceiptHook::new(
        batch_storage_dyn.clone(),
        batch_proofs.clone(),
    ))
    .retry(RetryConfig::default());

    let (chunk_prover, acct_prover) = if ext.dev_native_prover {
        info!(parent: tasks_span, "EE chunk + acct provers: native host (dev/test only)");
        let chunk = chunk_builder.native(EeChunkProgram::native_host());
        let acct_program = EeAcctProgram::new(EeChunkProgram::test_predicate_key());
        let acct = acct_builder.native(acct_program.native_host());
        (chunk, acct)
    } else {
        #[cfg(feature = "sp1")]
        {
            let deadline_secs = ext
                .sp1_proof_deadline_secs
                .unwrap_or(super::DEFAULT_SP1_DEADLINE_SECS);
            let deadline = std::time::Duration::from_secs(deadline_secs);
            info!(parent: tasks_span, deadline_secs, "sp1 EE prover deadline configured");
            let sp1_config = SP1HostConfig::default().with_deadline(deadline);
            let chunk_host: SP1Host = (**alpen_chunk_host(sp1_config.clone()).await).clone();
            let acct_host: SP1Host = (**alpen_acct_host(sp1_config).await).clone();
            (
                chunk_builder.remote(chunk_host),
                acct_builder.remote(acct_host),
            )
        }
        #[cfg(not(feature = "sp1"))]
        {
            return Err(eyre::eyre!(
                "remote SP1 prover is not compiled in; pass --dev-native-prover \
                    or build with the `sp1` feature"
            ));
        }
    };

    let prover_tick = std::time::Duration::from_secs(5);
    let chunk_handle = ProverServiceBuilder::new(chunk_prover)
        .tick_interval(prover_tick)
        .launch(service_executor)
        .await
        .map_err(|e| eyre::eyre!("launching chunk prover service: {e}"))?;
    let acct_handle = ProverServiceBuilder::new(acct_prover)
        .tick_interval(prover_tick)
        .launch(service_executor)
        .await
        .map_err(|e| eyre::eyre!("launching acct prover service: {e}"))?;

    let batch_prover = Arc::new(PaasBatchProver::new(
        chunk_handle,
        acct_handle,
        batch_storage_dyn,
        batch_proofs,
    ));

    info!(parent: tasks_span, "EE chunk + acct paas provers started");

    Ok(batch_prover)
}
