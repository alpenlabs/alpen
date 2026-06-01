//! Reth node for the Alpen codebase.

mod dummy_ol_client;
mod genesis;
mod gossip;
#[cfg(feature = "sequencer")]
mod header_summary;
mod ol_client;
#[cfg(feature = "sequencer")]
mod payload_builder;
#[cfg(feature = "sequencer")]
mod prover;
mod rpc_client;
mod service_executor;
mod services;

#[cfg(feature = "sequencer")]
use std::{env, process, sync::Arc, time::Duration};

use alpen_chainspec::{chain_value_parser, AlpenChainSpecParser};
use alpen_ee_common::{
    chain_status_checked, BatchStorage, BlockNumHash, ExecBlockStorage, OLClient, Storage,
};
use alpen_ee_config::{AlpenEeConfig, AlpenEeParams};
use alpen_ee_database::init_db_storage;
use alpen_ee_engine::{create_engine_control_task, sync_chainstate_to_engine, AlpenRethExecEngine};
#[cfg(feature = "sequencer")]
use alpen_ee_exec_chain::init_exec_chain_state_from_storage;
#[cfg(feature = "sequencer")]
use alpen_ee_genesis::ensure_finalized_exec_chain_genesis;
use alpen_ee_genesis::{ensure_batch_genesis, ensure_genesis_ee_account_state};
use alpen_ee_ol_tracker::init_ol_tracker_state;
use alpen_ee_rpc_server::{AlpenEeRpcServer, EeRpcServer};
#[cfg(feature = "sequencer")]
use alpen_ee_sequencer::{
    block_builder_task, build_ol_chain_tracker, init_ol_chain_tracker_state, BlockBuilderConfig,
};
use alpen_ee_sequencer::{init_batch_builder_state, init_lifecycle_state};
use alpen_reth_evm::evm::AlpenEvmFactory;
#[cfg(feature = "sequencer")]
use alpen_reth_exex::{AccessedStateGenerator, StateDiffGenerator};
use alpen_reth_node::{
    args::AlpenNodeArgs, AlpenEthereumNode, AlpenGossipProtocolHandler, AlpenGossipState,
};
#[cfg(feature = "sequencer")]
use bitcoind_async_client::{
    corepc_types::bitcoin::{
        key::Keypair,
        secp256k1::{Secp256k1, SecretKey},
    },
    traits::Wallet as _,
    Auth, Client as BtcClient,
};
use clap::{ArgAction, Parser};
use eyre::Context;
use reth_chainspec::ChainSpec;
use reth_cli_commands::{launcher::FnLauncher, node::NodeCommand};
use reth_cli_runner::{tokio_runtime, CliRunner};
use reth_cli_util::sigsegv_handler;
use reth_network::{protocol::IntoRlpxSubProtocol, NetworkProtocols};
use reth_node_builder::{NodeBuilder, WithLaunchContext};
use reth_provider::CanonStateSubscriptions;
use strata_acct_types::AccountId;
use strata_bridge_params::{BridgeParams, DEFAULT_DENOMINATION_SATS, DEFAULT_MAX_WITHDRAWAL_SATS};
#[cfg(feature = "sequencer")]
use strata_btcio::{
    broadcaster::BroadcasterBuilder, writer::chunked_envelope::create_chunked_envelope_task,
    BtcioParams,
};
use strata_common::healthz::{start_health_check_server, HealthCheckState};
#[cfg(feature = "sequencer")]
use strata_config::btcio::{FeePolicy, L1FeePolicyConfig, MempoolExplorerFeePolicy, WriterConfig};
use strata_identifiers::{EpochCommitment, OLBlockId};
use strata_l1_txfmt::MagicBytes;
use strata_logging::{init_logging_from_config, LoggingInitConfig};
use strata_predicate::PredicateKey;
use strata_primitives::{buf::Buf32, L1Height};
use tokio::sync::{mpsc, watch};
use tracing::{error, info};

#[cfg(feature = "sequencer")]
mod sequencer_imports {
    pub(super) use alpen_ee_common::{ChunkWitnessExtractFn, ChunkWitnessRecord};
    pub(super) use alpen_ee_da_provider::{ChunkedEnvelopeDaProvider, StateDiffBlobProvider};
    pub(super) use alpen_reth_witness::RangeWitnessExtractor;
    pub(super) use strata_paas::{
        ProverBuilder, ProverServiceBuilder, ReceiptStore, RetryConfig, TaskStore,
    };
    pub(super) use strata_proofimpl_alpen_acct::EeAcctProgram;
    pub(super) use strata_proofimpl_alpen_chunk::EeChunkProgram;
    #[cfg(feature = "sp1")]
    pub(super) use strata_zkvm_hosts::sp1::{alpen_acct_host, alpen_chunk_host};
    #[cfg(feature = "sp1")]
    pub(super) use zkaleido_sp1_host::{SP1Host, SP1HostConfig};

    pub(super) use crate::{
        header_summary::RethHeaderSummaryProvider,
        payload_builder::AlpenRethPayloadEngine,
        prover::{
            AcctRangeWitnessFn, AcctReceiptHook, AcctSpec, ChunkReceiptHook, ChunkSpec,
            EeBatchProofDbManager, EeChunkReceiptStore, EeProverTaskDbManager, PaasBatchProver,
        },
    };
}

#[cfg(feature = "sequencer")]
use sequencer_imports::*;

use crate::{
    dummy_ol_client::DummyOLClient,
    genesis::ee_genesis_block_info,
    gossip::{create_gossip_task, GossipConfig},
    ol_client::OLClientKind,
    rpc_client::RpcOLClient,
    service_executor::ServiceExecutor,
};

/// Environment variable for overriding the default EE block time.
#[cfg(feature = "sequencer")]
const ALPEN_EE_BLOCK_TIME_MS_ENV_VAR: &str = "ALPEN_EE_BLOCK_TIME_MS";

const DEFAULT_HEALTH_CHECK_HOST: &str = "0.0.0.0";
const DEFAULT_HEALTH_CHECK_PORT: u16 = 8080;

/// Default end-to-end deadline applied to the SP1 prover network for the EE
/// chunk + acct provers when `--sp1-proof-deadline-secs` is not set. Chosen
/// to comfortably cover chunk/acct proofs while still failing fast on stuck
/// requests.
#[cfg(all(feature = "sequencer", feature = "sp1"))]
const DEFAULT_SP1_DEADLINE_SECS: u64 = 4 * 60 * 60;

fn main() {
    sigsegv_handler::install();

    // Enable backtraces unless a RUST_BACKTRACE value has already been explicitly provided.
    if env::var_os("RUST_BACKTRACE").is_none() {
        // SAFETY: fine to set this in a non-async context.
        unsafe { env::set_var("RUST_BACKTRACE", "1") };
    }

    let mut command = NodeCommand::<AlpenChainSpecParser, AdditionalConfig>::parse();

    // use provided alpen chain spec
    command.chain = command.ext.custom_chain.clone();
    // enable engine api v4
    command.engine.accept_execution_requests_hash = true;
    // allow chain fork blocks to be created
    command
        .engine
        .always_process_payload_attributes_on_canonical_head = true;

    if let Err(err) = run(
        command,
        |builder: WithLaunchContext<NodeBuilder<Arc<reth_db::DatabaseEnv>, ChainSpec>>,
         ext: AdditionalConfig| async move {
            let service_executor = ServiceExecutor::from_reth(builder.task_executor().clone());
            let health_check_state = HealthCheckState::new();
            let health_check_addr = format!("{}:{}", ext.health_check_host, ext.health_check_port);
            let _health_check_handle =
                start_health_check_server(health_check_addr.clone(), health_check_state.clone())
                    .await
                    .context("failed to start health check server")?;
            info!(%health_check_addr, "health check server started");

            // --- CONFIGS ---

            // Resolve withdrawal cap: 0 → no cap, omitted → default 10 BTC.
            let resolved_max_withdrawal = match ext.max_withdrawal_amount {
                Some(0) => None,
                Some(v) => Some(v),
                None => Some(DEFAULT_MAX_WITHDRAWAL_SATS),
            };

            let datadir = builder.config().datadir().data_dir().to_path_buf();

            // TODO(STR-2982): read config, params from file
            let genesis_info = ee_genesis_block_info(&ext.custom_chain);

            // TODO(STR-3675): this must also be read from the params file
            // TODO(STR-3675): define how we want to deterministically generate the AccountId
            const ALPEN_EE_ACCOUNT_ID: AccountId = AccountId::new([1u8; 32]);

            info!(blockhash=%genesis_info.blockhash(), "EE genesis info");
            let params = AlpenEeParams::new(
                ALPEN_EE_ACCOUNT_ID,
                genesis_info.blockhash(),
                genesis_info.stateroot(),
                genesis_info.blocknum(),
            );

            info!(?params, sequencer = ext.sequencer, "Starting EE Node");

            // Resolve btcio writer config up front so flag misuse surfaces before I/O.
            #[cfg(feature = "sequencer")]
            let writer_config = if ext.sequencer {
                let cfg = Arc::new(resolve_writer_config(&ext)?);
                log_writer_config(&cfg);
                Some(cfg)
            } else {
                None
            };

            // OL client URL is not used when dummy_ol_client is enabled
            let ol_client_url = ext.ol_client_url.clone().unwrap_or_default();

            let config = Arc::new(AlpenEeConfig::new(
                params,
                PredicateKey::always_accept(),
                ol_client_url,
                ext.sequencer_http.clone(),
                ext.db_retry_count,
            ));

            #[cfg(feature = "sequencer")]
            let block_builder_config = block_builder_config_from_env(ext.sequencer)?;

            #[cfg(feature = "sequencer")]
            let sequencer_privkey = sequencer_privkey_from_env(ext.sequencer)?;

            #[cfg(feature = "sequencer")]
            // NOTE: ATM we reuse `SEQUENCER_PRIVATE_KEY` for both gossip
            // package signing and EE DA reveal tapscript signing. That is
            // operationally convenient for now, but it couples network
            // identity with Bitcoin DA spend authority. Should we split this
            // into a dedicated DA reveal signing key/config?
            let sequencer_keypair = match sequencer_privkey.as_ref() {
                Some(privkey) => Some(sequencer_bitcoin_keypair(privkey)?),
                None => None,
            };

            let gossip_config = {
                #[cfg(feature = "sequencer")]
                {
                    GossipConfig {
                        sequencer_pubkey: ext.sequencer_pubkey,
                        sequencer_enabled: ext.sequencer,
                        sequencer_privkey,
                    }
                }

                #[cfg(not(feature = "sequencer"))]
                {
                    GossipConfig {
                        sequencer_pubkey: ext.sequencer_pubkey,
                        sequencer_enabled: false,
                    }
                }
            };

            // --- INITIALIZE STATE ---

            let dbs = init_db_storage(&datadir, config.db_retry_count())
                .context("failed to load alpen database")?;

            let db_pool = threadpool::Builder::new()
                .num_threads(8)
                .thread_name("ee-db-pool".into())
                .build();
            let storage: Arc<_> = dbs.node_storage(db_pool.clone()).into();

            let ol_client = if ext.dummy_ol_client {
                use strata_identifiers::Buf32;
                use strata_primitives::EpochCommitment;
                let genesis_epoch = EpochCommitment::new(0, 0, OLBlockId::from(Buf32([1; 32])));
                info!(target: "alpen-client", "Using dummy OL client (no real OL connection)");
                OLClientKind::Dummy(DummyOLClient { genesis_epoch })
            } else {
                let ol_url = ext.ol_client_url.as_ref().ok_or_else(|| {
                    eyre::eyre!("--ol-client-url is required when not using --dummy-ol-client")
                })?;
                if ext.sequencer && ext.ol_submit_url.is_none() {
                    eyre::bail!(
                        "--ol-submit-url is required with --sequencer when not using \
                         --dummy-ol-client"
                    );
                }
                OLClientKind::Rpc(
                    RpcOLClient::try_new(
                        config.params().account_id(),
                        ol_url,
                        ext.ol_submit_url.as_deref(),
                        ext.ol_submit_bearer_token.as_deref(),
                    )
                    .map_err(|e| eyre::eyre!("failed to create OL client: {e}"))?,
                )
            };
            let ol_client = Arc::new(ol_client);

            // Fetch the genesis epoch commitment from the OL client once at startup.
            let genesis_epoch = ol_client
                .account_genesis_epoch()
                .await
                .context("failed to fetch account genesis epoch from OL")?;

            ensure_genesis(config.as_ref(), &genesis_epoch, storage.as_ref())
                .await
                .context("genesis should not fail")?;

            let ol_chain_status = chain_status_checked(ol_client.as_ref())
                .await
                .context("cannot fetch OL chain status")?;

            let ol_tracker_state = init_ol_tracker_state(ol_chain_status, storage.as_ref())
                .await
                .context("ol tracker state initialization should not fail")?;

            #[cfg(feature = "sequencer")]
            let ol_chain_tracker_state =
                init_ol_chain_tracker_state(storage.as_ref(), ol_client.as_ref())
                    .await
                    .context("ol chain tracker state initialization should not fail")?;

            #[cfg(feature = "sequencer")]
            let exec_chain_state = init_exec_chain_state_from_storage(storage.as_ref())
                .await
                .context("exec chain state initialization should not fail")?;

            let initial_preconf_head = {
                #[cfg(feature = "sequencer")]
                {
                    if ext.sequencer {
                        exec_chain_state.tip_blocknumhash()
                    } else {
                        // In non-sequencer mode, we only have the hash from OL tracker.
                        // Use block number 0 as initial value; it will be updated by gossip.
                        let hash = ol_tracker_state.best_ee_state().last_exec_blkid();
                        BlockNumHash::new(hash, 0)
                    }
                }
                #[cfg(not(feature = "sequencer"))]
                {
                    // In non-sequencer mode, we only have the hash from OL tracker.
                    // Use block number 0 as initial value; it will be updated by gossip.
                    let hash = ol_tracker_state.best_ee_state().last_exec_blkid();
                    BlockNumHash::new(hash, 0)
                }
            };

            let batch_builder_state = init_batch_builder_state(storage.as_ref())
                .await
                .context("batch builder state initialization should not fail")?;

            let batch_lifecycle_state = init_lifecycle_state(storage.as_ref())
                .await
                .context("batch lifecycle state initialization should not fail")?;
            // --- INITIALIZE SERVICES ---

            // Create gossip channel before building the node so we can register it early
            let (gossip_tx, gossip_rx) = mpsc::unbounded_channel();

            // Create preconf channel for p2p head block gossip -> engine control integration
            // This channel sends block hash and number received from peers to the engine control
            // task
            let (preconf_tx, preconf_rx) = watch::channel(initial_preconf_head);

            let ol_tracker = services::ol_tracker::start_ol_tracker_service(
                ol_tracker_state,
                genesis_epoch.epoch(),
                storage.clone(),
                ol_client.clone(),
                ext.dev_track_latest_epoch,
                &service_executor,
            )
            .await
            .map_err(|e| eyre::eyre!("failed to start ol tracker service: {e}"))?;

            let evm_factory = AlpenEvmFactory::from_bridge_params(
                &BridgeParams::new(ext.bridge_denomination, resolved_max_withdrawal)
                    .expect("invalid withdrawal params"),
            );
            let node_args = AlpenNodeArgs {
                sequencer_http: ext.sequencer_http.clone(),
                evm_factory,
            };

            let consensus_watcher = ol_tracker.consensus_watcher();
            let status_watcher = ol_tracker.ol_status_watcher();

            let mut node_builder = builder
                .node(AlpenEthereumNode::new(node_args))
                // Register Alpen gossip RLPx subprotocol
                .on_component_initialized({
                    let gossip_tx = gossip_tx.clone();
                    move |node| {
                        // Add the custom RLPx subprotocol before node fully starts
                        // See: crates/reth/node/src/gossip/
                        let handler =
                            AlpenGossipProtocolHandler::new(AlpenGossipState::new(gossip_tx));
                        node.components
                            .network
                            .add_rlpx_sub_protocol(handler.into_rlpx_sub_protocol());
                        info!(target: "alpen-gossip", "Registered Alpen gossip RLPx subprotocol");
                        Ok(())
                    }
                });

            // Install state diff exex for sequencer DA.
            // The exex persists per-block state diffs that the blob provider reads.
            #[cfg(feature = "sequencer")]
            if ext.sequencer {
                node_builder = node_builder.install_exex("state_diffs", {
                    let state_diff_db = dbs.witness_db();
                    |ctx| async { Ok(StateDiffGenerator::new(ctx, state_diff_db).start()) }
                });
                info!(target: "alpen-client", "installed StateDiffGenerator exex for DA");

                // Per-block accessed-state capture. Re-executes each
                // committed block with a `CacheDBProvider` to record the
                // read set + bytecodes, which the chunk-builder consumes
                // at chunk-seal time to skip its own re-execution loop.
                node_builder = node_builder.install_exex("accessed_state", {
                    let accessed_state_store = storage.clone();
                    |ctx| async {
                        Ok(AccessedStateGenerator::new(ctx, accessed_state_store).start())
                    }
                });
                info!(target: "alpen-client", "installed AccessedStateGenerator exex");
            }

            node_builder = node_builder.extend_rpc_modules({
                let consensus_watcher = consensus_watcher.clone();
                let storage = storage.clone();
                move |ctx| {
                    let provider = ctx.provider().clone();
                    let ee_rpc_server =
                        EeRpcServer::new(provider, consensus_watcher, storage.clone());
                    ctx.modules.merge_configured(ee_rpc_server.into_rpc())?;
                    Ok(())
                }
            });

            let handle = node_builder.launch().await?;

            let node = handle.node;

            // Sync chainstate to engine for sequencer nodes before starting other tasks
            #[cfg(feature = "sequencer")]
            if ext.sequencer {
                let engine = AlpenRethExecEngine::new(node.beacon_engine_handle.clone());
                let storage_clone = storage.clone();
                let provider_clone = node.provider.clone();

                // Block on the async sync operation
                let sync_result =
                    sync_chainstate_to_engine(storage_clone.as_ref(), &provider_clone, &engine)
                        .await;

                if let Err(e) = sync_result {
                    error!(target: "alpen-client", error = ?e, "failed to sync chainstate to engine on startup");
                    return Err(eyre::eyre!("chainstate sync failed: {e}"));
                }

                info!(target: "alpen-client", "chainstate sync completed successfully");
            }

            let engine_control_task = create_engine_control_task(
                preconf_rx.clone(),
                consensus_watcher.clone(),
                node.provider.clone(),
                AlpenRethExecEngine::new(node.beacon_engine_handle.clone()),
            );

            // Subscribe to canonical state notifications for broadcasting new blocks
            let state_events = node.provider.subscribe_to_canonical_state();

            // Create gossip task for broadcasting new blocks
            let gossip_task =
                create_gossip_task(gossip_rx, state_events, preconf_tx.clone(), gossip_config);

            // Spawn critical tasks
            node.task_executor
                .spawn_critical("engine_control", engine_control_task);
            node.task_executor
                .spawn_critical("gossip_task", gossip_task);

            #[cfg(feature = "sequencer")]
            if ext.sequencer {
                // sequencer specific tasks

                use alpen_ee_common::{require_latest_batch, BlockNumHash, DaBlobSource};
                use alpen_ee_sequencer::{
                    backfill_missing_chunk_witnesses, chunk_witness_channel, chunk_witness_task,
                    create_batch_builder, create_batch_lifecycle_task,
                    create_update_submitter_task, BlockCountDataProvider, FixedBlockCountSealing,
                };
                let payload_engine = Arc::new(AlpenRethPayloadEngine::new(
                    node.payload_builder_handle.clone(),
                    node.beacon_engine_handle.clone(),
                ));

                let exec_chain_handle = services::exec_chain::start_exec_chain_service(
                    exec_chain_state,
                    preconf_tx.clone(),
                    storage.clone(),
                    consensus_watcher.clone(),
                    &service_executor,
                )
                .await
                .map_err(|e| eyre::eyre!("failed to start exec chain service: {e}"))?;

                let (ol_chain_tracker, ol_chain_tracker_task) = build_ol_chain_tracker(
                    ol_chain_tracker_state,
                    status_watcher.clone(),
                    ol_client.clone(),
                    storage.clone(),
                );

                let (latest_batch, _) = require_latest_batch(storage.as_ref()).await?;

                let batch_sealing_policy =
                    FixedBlockCountSealing::new(ext.batch_sealing_block_count);
                let block_data_provider = Arc::new(BlockCountDataProvider);

                // Chunk-spanning witness extractor. Single producer of
                // `ChunkWitnessRecord`: the background `chunk_witness_task`
                // invokes it once per chunk, off the batch builder's hot
                // path. The chunk prover's `ChunkSpec::fetch_input` reads
                // the persisted record from sled and has no extractor path
                // of its own; a missing record returns `TransientFailure`
                // and the task here retries on extraction errors (mainly
                // covering the race against the `AccessedStateGenerator`
                // exex).
                let range_witness_extractor = Arc::new(RangeWitnessExtractor::new(
                    node.provider.clone(),
                    storage.clone(),
                ));

                // `ChunkWitnessExtractFn` is the production-time hook: takes
                // chunk endpoints, returns the persisted `ChunkWitnessRecord`.
                // Reth's alloy `Header` / `Block` are RLP-encoded here so the
                // record stays Borsh-friendly for sled.
                let chunk_witness_extract_fn: Arc<ChunkWitnessExtractFn> = {
                    let extractor = range_witness_extractor.clone();
                    Arc::new(move |first_block, last_block| {
                        let first_b256 = alloy_primitives::B256::from(first_block.0);
                        let last_b256 = alloy_primitives::B256::from(last_block.0);
                        let data = extractor.extract_range_witness(first_b256, last_b256)?;
                        let prev_header_rlp = alloy_rlp::encode(&data.prev_header);
                        let blocks_rlp: Vec<Vec<u8>> =
                            data.blocks.iter().map(alloy_rlp::encode).collect();
                        Ok(ChunkWitnessRecord::new(
                            data.raw_partial_pre_state,
                            prev_header_rlp,
                            blocks_rlp,
                        ))
                    })
                };

                let (chunk_witness_tx, chunk_witness_rx) = chunk_witness_channel();
                let chunk_witness_store: Arc<dyn alpen_ee_common::ChunkWitnessStore> =
                    storage.clone();
                let chunk_witness_task_fut = chunk_witness_task(
                    chunk_witness_extract_fn,
                    chunk_witness_store,
                    chunk_witness_rx,
                );

                // Startup recovery for sealed-without-witness chunks.
                // Covers crash-mid-extraction (mpsc request lost with the
                // process) and any pre-existing chunks lacking a witness
                // row. Spawned critical alongside `ee_chunk_witness`:
                // witness assembly gates the entire proving pipeline, so
                // a panic in either path warrants taking the node down
                // rather than silently producing un-provable chunks.
                let chunk_witness_backfill_task = {
                    let batch_storage: Arc<dyn BatchStorage> = storage.clone();
                    let witness_store: Arc<dyn alpen_ee_common::ChunkWitnessStore> =
                        storage.clone();
                    let tx = chunk_witness_tx.clone();
                    async move {
                        if let Err(e) = backfill_missing_chunk_witnesses(
                            batch_storage.as_ref(),
                            witness_store.as_ref(),
                            &tx,
                        )
                        .await
                        {
                            error!(error = %e, "chunk witness backfill failed at startup");
                        }
                    }
                };

                let (batch_builder_handle, batch_builder_task) = create_batch_builder(
                    latest_batch.id(),
                    BlockNumHash::new(genesis_info.blockhash().0.into(), genesis_info.blocknum()),
                    batch_builder_state,
                    preconf_rx,
                    block_data_provider,
                    batch_sealing_policy,
                    storage.clone(),
                    storage.clone(),
                    exec_chain_handle.clone(),
                    Some(chunk_witness_tx),
                );

                // --- DA pipeline ---
                //
                // clap `requires_all` on --sequencer guarantees all DA args are present.
                let magic_bytes = ext.ee_da_magic_bytes.expect("enforced by clap");
                let btc_url = ext.btc_rpc_url.as_ref().expect("enforced by clap");
                let btc_user = ext.btc_rpc_user.as_ref().expect("enforced by clap");
                let btc_pass = ext.btc_rpc_password.as_ref().expect("enforced by clap");

                // Create BtcioParams directly from CLI args.
                let btcio_params =
                    BtcioParams::new(ext.l1_reorg_safe_depth, magic_bytes, ext.genesis_l1_height);

                // Bitcoin RPC client.
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
                    target: "alpen-client",
                    retry_count = ext.btcio_retry_count,
                    retry_interval_ms = ext.btcio_retry_interval,
                    "btcio Bitcoin RPC retry policy configured",
                );

                // Sequencer address from bitcoin wallet.
                let sequencer_address = btc_client
                    .get_new_address()
                    .await
                    .map_err(|e| eyre::eyre!("failed to get sequencer address: {e}"))?;

                // Wrap raw DBs in ops using the shared DB threadpool.
                let broadcast_ops = Arc::new(dbs.broadcast_ops(db_pool.clone()));
                let envelope_ops = Arc::new(dbs.chunked_envelope_ops(db_pool));

                // Launch broadcaster service and create chunked envelope task.
                let broadcast_poll_interval = 5_000;

                let broadcast_handle = Arc::new(
                    BroadcasterBuilder::new(
                        btc_client.clone(),
                        broadcast_ops.clone(),
                        btcio_params,
                    )
                    .with_broadcast_poll_interval_ms(broadcast_poll_interval)
                    .launch(&service_executor)
                    .await
                    .map_err(|e| eyre::eyre!("starting broadcaster service: {e}"))?,
                );

                let writer_config = writer_config
                    .clone()
                    .expect("writer_config resolved at startup when --sequencer is set");
                let sequencer_keypair = sequencer_keypair.ok_or_else(|| {
                    eyre::eyre!("EE sequencer DA reveal signing needs sequencer Keypair")
                })?;
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

                let header_summary =
                    Arc::new(RethHeaderSummaryProvider::new(node.provider.clone()));

                let da_context_db = dbs.da_context_db();
                let blob_provider: Arc<dyn DaBlobSource> = Arc::new(StateDiffBlobProvider::new(
                    storage.clone(),
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

                // Spawn btcio tasks.
                node.task_executor
                    .spawn_critical("chunked_envelope_watcher", envelope_watcher_task);

                info!(target: "alpen-client", "btcio DA pipeline started");

                // EE chunk + acct paas provers. Both use SP1 remote
                // proving (production); native is dev-only via the
                // proofimpl crates' `native_host()` for tests.
                //
                // Storage layout (sled-backed, own sled db under
                // `<datadir>/sled` — fully separate from OL's; the
                // prover trees live alongside the EE node trees):
                //   - `task_store` — shared across both provers; task keys carry a kind tag
                //     (`b'c'`/`b'a'`) so chunk and batch entries don't collide in one tree.
                //   - `chunk_receipts` — chunk prover writes (via paas auto-store); acct
                //     `fetch_input` reads back.
                //   - `batch_proofs` — outer-proof store keyed by `BatchId`; outer hook writes, OL
                //     submission reads.
                //
                // All backed by `EeProverDbSled`; see
                // `alpen_ee_database::sleddb::prover_db` for schemas.
                let prover_db = dbs.prover_db();
                let task_store: Arc<dyn TaskStore> =
                    Arc::new(EeProverTaskDbManager::new(prover_db.clone()));
                let chunk_receipts: Arc<dyn ReceiptStore> =
                    Arc::new(EeChunkReceiptStore::new(prover_db.clone()));
                let batch_proofs = Arc::new(EeBatchProofDbManager::new(prover_db));
                let batch_storage_dyn: Arc<dyn BatchStorage> = storage.clone();

                let genesis = {
                    use alpen_reth_exex::alloy2reth::IntoRspChainConfig as _;
                    ext.custom_chain.genesis().config.clone().into_rsp()
                };

                let bridge_params =
                    BridgeParams::new(ext.bridge_denomination, resolved_max_withdrawal)
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
                    let extractor = range_witness_extractor.clone();
                    Arc::new(move |first_block, last_block| {
                        extractor.extract_range_witness(first_block, last_block)
                    })
                };

                let acct_builder = ProverBuilder::new(AcctSpec::new(
                    chunk_receipts.clone(),
                    batch_storage_dyn.clone(),
                    storage.clone(),
                    btc_client.clone(),
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

                // Dev/test escape hatch: use zkaleido NativeHost instead of
                // the SP1 remote host. This skips real Groth16 proving and
                // the need for compiled guest ELFs — only safe for
                // functional tests. The acct program is wired with the
                // chunk program's deterministic test predicate key so the
                // native-host Schnorr signature actually verifies.
                let (chunk_prover, acct_prover) = if ext.dev_native_prover {
                    info!(
                        target: "alpen-client",
                        "EE chunk + acct provers: native host (dev/test only)"
                    );
                    let chunk = chunk_builder.native(EeChunkProgram::native_host());
                    let acct_program = EeAcctProgram::new(EeChunkProgram::test_predicate_key());
                    let acct = acct_builder.native(acct_program.native_host());
                    (chunk, acct)
                } else {
                    #[cfg(feature = "sp1")]
                    {
                        let deadline_secs = ext
                            .sp1_proof_deadline_secs
                            .unwrap_or(DEFAULT_SP1_DEADLINE_SECS);
                        let deadline = Duration::from_secs(deadline_secs);
                        info!(
                            target: "alpen-client",
                            deadline_secs,
                            "sp1 EE prover deadline configured"
                        );
                        let sp1_config = SP1HostConfig::default().with_deadline(deadline);
                        let chunk_host: SP1Host =
                            (**alpen_chunk_host(sp1_config.clone()).await).clone();
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

                let prover_tick = Duration::from_secs(5);
                let chunk_handle = ProverServiceBuilder::new(chunk_prover)
                    .tick_interval(prover_tick)
                    .launch(&service_executor)
                    .await
                    .map_err(|e| eyre::eyre!("launching chunk prover service: {e}"))?;
                let acct_handle = ProverServiceBuilder::new(acct_prover)
                    .tick_interval(prover_tick)
                    .launch(&service_executor)
                    .await
                    .map_err(|e| eyre::eyre!("launching acct prover service: {e}"))?;

                let batch_prover = Arc::new(PaasBatchProver::new(
                    chunk_handle,
                    acct_handle,
                    batch_storage_dyn,
                    batch_proofs,
                ));

                info!(target: "alpen-client", "EE chunk + acct paas provers started (SP1 remote)");

                let (batch_lifecycle_handle, batch_lifecycle_task) = create_batch_lifecycle_task(
                    None,
                    batch_lifecycle_state,
                    batch_builder_handle.latest_batch_watcher(),
                    batch_da_provider,
                    batch_prover.clone(),
                    storage.clone(),
                    blob_provider,
                    da_context_db,
                );

                let update_submitter_task = create_update_submitter_task(
                    ol_client,
                    storage.clone(),
                    storage.clone(),
                    batch_prover,
                    batch_lifecycle_handle.latest_proof_ready_watcher(),
                    status_watcher,
                );

                node.task_executor
                    .spawn_critical("ol_chain_tracker", ol_chain_tracker_task);
                node.task_executor.spawn_critical(
                    "block_assembly",
                    block_builder_task(
                        block_builder_config,
                        exec_chain_handle,
                        ol_chain_tracker,
                        payload_engine,
                        storage.clone(),
                    ),
                );

                node.task_executor
                    .spawn_critical("ee_batch_builder", batch_builder_task);
                node.task_executor
                    .spawn_critical("ee_chunk_witness", chunk_witness_task_fut);
                node.task_executor
                    .spawn_critical("ee_chunk_witness_backfill", chunk_witness_backfill_task);
                node.task_executor
                    .spawn_critical("ee_batch_lifecycle", batch_lifecycle_task);
                node.task_executor
                    .spawn_critical("ee_update_submitter", update_submitter_task);
            }

            health_check_state.mark_ready();
            handle.node_exit_future.await
        },
    ) {
        eprintln!("Error: {err:?}");
        process::exit(1);
    }
}

/// Our custom cli args extension that adds one flag to reth default CLI.
#[derive(Debug, clap::Parser)]
pub struct AdditionalConfig {
    /// Set the minimum log level.
    ///
    /// -v      Errors
    /// -vv     Warnings
    /// -vvv    Info
    /// -vvvv   Debug
    /// -vvvvv  Traces (warning: very verbose!)
    #[arg(
        short,
        long,
        action = ArgAction::Count,
        global = true,
        verbatim_doc_comment,
        help_heading = "Display"
    )]
    pub verbosity: u8,

    /// Silence all log output.
    #[arg(
        long,
        alias = "silent",
        short = 'q',
        global = true,
        help_heading = "Display"
    )]
    pub quiet: bool,

    /// OTLP gRPC endpoint for the OpenTelemetry collector.
    ///
    /// When set, `strata-logging` builds a tracer provider against this
    /// endpoint. Metrics stay on Reth's native recorder and Prometheus
    /// endpoint; use Reth's `--metrics` flag for `/metrics`.
    /// Falls back to the standard `OTEL_EXPORTER_OTLP_ENDPOINT` env var
    /// when the flag isn't passed.
    #[arg(long, env = "OTEL_EXPORTER_OTLP_ENDPOINT")]
    pub otlp_url: Option<String>,

    /// Optional service label suffix appended to the OpenTelemetry
    /// `service.name` resource attribute (e.g. `prod`, `dev`,
    /// `staging-v2`). Mirrors `bin/strata`'s `--service-label`.
    #[arg(long)]
    pub service_label: Option<String>,

    /// The chain this node is running.
    ///
    /// Possible values are either a built-in chain or the path to a chain specification file.
    /// Cannot override existing `chain` arg, so this is a workaround.
    #[arg(
        long,
        value_name = "CHAIN_OR_PATH",
        default_value = "testnet",
        value_parser = chain_value_parser,
        required = false,
    )]
    pub custom_chain: Arc<ChainSpec>,

    /// Rpc of sequencer's reth node to forward transactions to.
    #[arg(long, required = false)]
    pub sequencer_http: Option<String>,

    /// URL of OL node RPC (can be either `http[s]://` or `ws[s]://`).
    /// Required unless `--dummy-ol-client` is specified.
    #[arg(long)]
    pub ol_client_url: Option<String>,

    /// URL of the authenticated OL transaction submission RPC.
    /// Required with `--sequencer` unless `--dummy-ol-client` is specified.
    #[arg(long)]
    pub ol_submit_url: Option<String>,

    /// Bearer token for the authenticated OL transaction submission RPC.
    #[arg(long, env = "STRATA_SUBMIT_RPC_TOKEN")]
    pub ol_submit_bearer_token: Option<String>,

    /// Use a dummy OL client instead of connecting to a real OL node.
    /// This is useful for testing EE functionality in isolation.
    ///
    /// NOTE: This is intentionally separate from OL-EE integration tests which
    /// need the real OL RPC client. The dummy client is only for EE-specific
    /// tests that don't need OL interaction.
    #[arg(long, default_value_t = false)]
    pub dummy_ol_client: bool,

    /// Host for the HTTP health check endpoint.
    #[arg(long, default_value = DEFAULT_HEALTH_CHECK_HOST)]
    pub health_check_host: String,

    /// Port for the HTTP health check endpoint.
    #[arg(long, default_value_t = DEFAULT_HEALTH_CHECK_PORT)]
    pub health_check_port: u16,

    #[arg(long, required = false)]
    pub db_retry_count: Option<u16>,

    /// Run the node as a sequencer. Requires the `sequencer` feature,
    /// a `SEQUENCER_PRIVATE_KEY` environment variable, and all DA-related
    /// arguments (`--ee-da-magic-bytes`, `--btc-rpc-url`, `--btc-rpc-user`,
    /// `--btc-rpc-password`).
    #[arg(
        long,
        default_value_t = false,
        requires_all = ["ee_da_magic_bytes", "btc_rpc_url", "btc_rpc_user", "btc_rpc_password"],
    )]
    pub sequencer: bool,

    /// Sequencer's public key (hex-encoded, 32 bytes) for signature validation.
    #[arg(long, required = true, value_parser = parse_buf32)]
    pub sequencer_pubkey: Buf32,

    // --- DA Configuration ---
    /// Magic bytes (hex-encoded, 4 bytes) for tagging EE DA envelope transactions.
    /// Example: `ALPN`.
    #[arg(long, required = false, value_parser = parse_magic_bytes)]
    pub ee_da_magic_bytes: Option<MagicBytes>,

    /// Bitcoin Core RPC URL. Required when `--sequencer` is set.
    #[arg(long, required = false)]
    pub btc_rpc_url: Option<String>,

    /// Bitcoin Core RPC username. Required when `--sequencer` is set.
    #[arg(long, required = false)]
    pub btc_rpc_user: Option<String>,

    /// Bitcoin Core RPC password. Required when `--sequencer` is set.
    #[arg(long, required = false)]
    pub btc_rpc_password: Option<String>,

    /// L1 reorg safe depth (number of confirmations for finality).
    #[arg(long, default_value = "6")]
    pub l1_reorg_safe_depth: u32,

    /// Genesis L1 block height (the first L1 block the rollup cares about).
    #[arg(long, default_value = "0")]
    pub genesis_l1_height: L1Height,

    /// Number of blocks per batch before sealing.
    /// Lower values seal batches more frequently (useful for testing).
    #[arg(long, default_value = "100")]
    pub batch_sealing_block_count: u64,

    /// Bridge denomination in satoshis (1 BTC default).
    #[arg(long, default_value_t = DEFAULT_DENOMINATION_SATS)]
    pub bridge_denomination: u64,

    /// Maximum withdrawal amount in satoshis.
    ///
    /// When omitted, defaults to 1_000_000_000 (10 BTC) at runtime.
    /// Pass 0 to disable the cap entirely. Kept as `Option` (no
    /// `default_value`) so we can distinguish "not set" (→ safe default)
    /// from an explicit value.
    #[arg(long)]
    pub max_withdrawal_amount: Option<u64>,

    /// Use the zkaleido `NativeHost` for the EE chunk + acct provers
    /// instead of the SP1 remote host.
    ///
    /// Dev/test only: skips real Groth16 proving and the compiled guest
    /// ELFs. Functional tests enable this so the sequencer can start
    /// without the SP1 prover ELFs present on disk.
    #[arg(long, default_value_t = false)]
    pub dev_native_prover: bool,

    /// Have the OL chain tracker advance against the latest completed OL
    /// epoch in the connected Strata node instead of the canonical
    /// `confirmed` epoch (CSM-based). Dev/test only. Useful when the CSM
    /// checkpoint pipeline can't keep up with rapid SAU emission and would
    /// otherwise stall the EE block builder's inbox-message fetch.
    #[arg(long, default_value_t = false)]
    pub dev_track_latest_epoch: bool,

    /// End-to-end deadline (seconds) passed to the SP1 prover network on
    /// every chunk/acct proof request. Only used with the remote SP1
    /// backend. When unset, a built-in default is applied (see
    /// `DEFAULT_SP1_DEADLINE_SECS`).
    #[arg(long, required = false)]
    pub sp1_proof_deadline_secs: Option<u64>,

    /// btcio writer fee policy: `bitcoind`, `fixed`, or `mempool`.
    #[cfg(feature = "sequencer")]
    #[arg(long, value_enum, default_value_t = BtcioFeePolicyArg::Bitcoind)]
    pub btcio_fee_policy: BtcioFeePolicyArg,

    /// Confirmation target for `bitcoind`; also the mempool fallback.
    #[cfg(feature = "sequencer")]
    #[arg(long, default_value = "1")]
    pub btcio_conf_target: u16,

    /// Fixed fee rate in sat/vB. Required when policy is `fixed`.
    #[cfg(feature = "sequencer")]
    #[arg(long)]
    pub btcio_fee_rate: Option<u64>,

    /// mempool.space-compatible base URL. Required when policy is `mempool`.
    #[cfg(feature = "sequencer")]
    #[arg(long)]
    pub btcio_mempool_base_url: Option<String>,

    /// Mempool fee tier when policy is `mempool`.
    #[cfg(feature = "sequencer")]
    #[arg(long, value_enum, default_value_t = BtcioMempoolTierArg::Fastest)]
    pub btcio_mempool_tier: BtcioMempoolTierArg,

    /// Max retries for Bitcoin RPC requests.
    #[cfg(feature = "sequencer")]
    #[arg(long, default_value_t = DEFAULT_BTCIO_RETRY_COUNT)]
    pub btcio_retry_count: u16,

    /// Bitcoin RPC retry interval in ms.
    #[cfg(feature = "sequencer")]
    #[arg(long, default_value_t = DEFAULT_BTCIO_RETRY_INTERVAL_MS)]
    pub btcio_retry_interval: u64,
}

impl AdditionalConfig {
    /// Returns an EnvFilter-compatible directive for CLI verbosity flags.
    fn verbosity_filter_directive(&self) -> Option<&'static str> {
        if self.quiet {
            return Some("off");
        }

        match self.verbosity {
            0 => None,
            1 => Some("error"),
            2 => Some("warn"),
            3 => Some("info"),
            4 => Some("debug"),
            _ => Some("trace"),
        }
    }
}

/// Run node with logging
/// based on reth::cli::Cli::run
fn run<L>(
    command: NodeCommand<AlpenChainSpecParser, AdditionalConfig>,
    launcher: L,
) -> eyre::Result<()>
where
    L: std::ops::AsyncFnOnce(
        WithLaunchContext<NodeBuilder<Arc<reth_db::DatabaseEnv>, ChainSpec>>,
        AdditionalConfig,
    ) -> eyre::Result<()>,
{
    if command.ext.sequencer && !cfg!(feature = "sequencer") {
        error!(
            target: "alpen-client",
            "Sequencer flag enabled but binary built without `sequencer` feature. Rebuild with default features or enable the `sequencer` feature."
        );
        eyre::bail!("sequencer feature not enabled at compile time");
    }

    // Build the tokio runtime ourselves so logging init can run inside its
    // context, then hand it to CliRunner. The OTLP tracing exporter requires
    // an active tokio handle when it is built.
    let rt = tokio_runtime()?;

    {
        let _g = rt.handle().enter();

        let mut extra_filter_directives =
            vec!["sp1_core_executor=warn", "jsonrpsee_server::server=warn"];
        if let Some(verbosity_filter) = command.ext.verbosity_filter_directive() {
            extra_filter_directives.push(verbosity_filter);
        }

        init_logging_from_config(LoggingInitConfig {
            service_base_name: "alpen-client",
            service_label: command.ext.service_label.as_deref(),
            otlp_url: command.ext.otlp_url.as_deref(),
            log_dir: None,
            log_file_prefix: None,
            json_format: None,
            default_log_prefix: "alpen-client",
            extra_filter_directives: &extra_filter_directives,
        });
    }

    let runner = CliRunner::from_runtime(rt);

    info!(target: "alpen-client", "logging initialized");

    let result = runner.run_command_until_exit(|ctx| {
        command.execute(
            ctx,
            FnLauncher::new::<AlpenChainSpecParser, AdditionalConfig>(launcher),
        )
    });

    // Flush OTLP tracing buffers before the process exits.
    strata_logging::finalize();

    result
}

/// Parse a hex-encoded string into a [`Buf32`].
fn parse_buf32(s: &str) -> eyre::Result<Buf32> {
    s.parse::<Buf32>()
        .map_err(|e| eyre::eyre!("Failed to parse hex string as Buf32: {e}"))
}

/// Parse a magic bytes string using the [`MagicBytes`] parser from `strata-l1-txfmt`.
fn parse_magic_bytes(s: &str) -> eyre::Result<MagicBytes> {
    s.parse::<MagicBytes>()
        .map_err(|e| eyre::eyre!("Failed to parse magic bytes: {e}"))
}

#[cfg(feature = "sequencer")]
fn sequencer_privkey_from_env(sequencer_enabled: bool) -> eyre::Result<Option<Buf32>> {
    if !sequencer_enabled {
        return Ok(None);
    }

    let privkey_str = env::var("SEQUENCER_PRIVATE_KEY").map_err(|_| {
        eyre::eyre!(
            "SEQUENCER_PRIVATE_KEY environment variable is required when running with --sequencer"
        )
    })?;

    let privkey = privkey_str
        .parse::<Buf32>()
        .map_err(|e| eyre::eyre!("Failed to parse SEQUENCER_PRIVATE_KEY as hex: {e}"))?;

    Ok(Some(privkey))
}

#[cfg(feature = "sequencer")]
fn sequencer_bitcoin_keypair(privkey: &Buf32) -> eyre::Result<Keypair> {
    let sk = SecretKey::from_slice(privkey.as_ref()).context("invalid sequencer private key")?;
    let secp = Secp256k1::signing_only();
    Ok(Keypair::from_secret_key(&secp, &sk))
}

// Mirrors `bitcoind-async-client`'s upstream defaults.
#[cfg(feature = "sequencer")]
const DEFAULT_BTCIO_RETRY_COUNT: u16 = 3;
#[cfg(feature = "sequencer")]
const DEFAULT_BTCIO_RETRY_INTERVAL_MS: u64 = 1_000;

/// CLI mirror of [`FeePolicy`].
#[cfg(feature = "sequencer")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum BtcioFeePolicyArg {
    Bitcoind,
    Fixed,
    Mempool,
}

/// CLI mirror of [`MempoolExplorerFeePolicy`].
#[cfg(feature = "sequencer")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum BtcioMempoolTierArg {
    Fastest,
    HalfHour,
    Hour,
    Economy,
    Minimum,
}

#[cfg(feature = "sequencer")]
impl From<BtcioMempoolTierArg> for MempoolExplorerFeePolicy {
    fn from(value: BtcioMempoolTierArg) -> Self {
        match value {
            BtcioMempoolTierArg::Fastest => Self::Fastest,
            BtcioMempoolTierArg::HalfHour => Self::HalfHour,
            BtcioMempoolTierArg::Hour => Self::Hour,
            BtcioMempoolTierArg::Economy => Self::Economy,
            BtcioMempoolTierArg::Minimum => Self::Minimum,
        }
    }
}

/// Builds [`WriterConfig`] from CLI flags. Empty-string mempool URL is
/// treated as absent so docker-compose `${VAR:-}` doesn't yield `Some("")`.
#[cfg(feature = "sequencer")]
fn resolve_writer_config(ext: &AdditionalConfig) -> eyre::Result<WriterConfig> {
    let mempool_base_url = ext
        .btcio_mempool_base_url
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    let fee_policy = match ext.btcio_fee_policy {
        BtcioFeePolicyArg::Bitcoind => FeePolicy::BitcoinD {
            conf_target: ext.btcio_conf_target,
        },
        BtcioFeePolicyArg::Fixed => {
            let fee_rate = ext.btcio_fee_rate.ok_or_else(|| {
                eyre::eyre!("--btcio-fee-rate is required when --btcio-fee-policy=fixed")
            })?;
            FeePolicy::Fixed { fee_rate }
        }
        BtcioFeePolicyArg::Mempool => {
            let base_url = mempool_base_url.clone().ok_or_else(|| {
                eyre::eyre!("--btcio-mempool-base-url is required when --btcio-fee-policy=mempool")
            })?;
            FeePolicy::MempoolExplorer {
                policy: ext.btcio_mempool_tier.into(),
                mempool_base_url: base_url,
                fallback_conf_target: ext.btcio_conf_target,
            }
        }
    };
    Ok(WriterConfig {
        l1_fee_policy_config: L1FeePolicyConfig::new(fee_policy),
        ..WriterConfig::default()
    })
}

#[cfg(feature = "sequencer")]
fn log_writer_config(cfg: &WriterConfig) {
    match cfg.fee_policy() {
        FeePolicy::BitcoinD { conf_target } => {
            info!(
                target: "alpen-client",
                policy = "bitcoind",
                conf_target,
                "btcio writer configured",
            );
        }
        FeePolicy::Fixed { fee_rate } => {
            info!(
                target: "alpen-client",
                policy = "fixed",
                fee_rate_sat_vb = fee_rate,
                "btcio writer configured",
            );
        }
        FeePolicy::MempoolExplorer {
            policy,
            mempool_base_url,
            fallback_conf_target,
        } => {
            info!(
                target: "alpen-client",
                policy = "mempool",
                tier = ?policy,
                base_url = %mempool_base_url,
                fallback_conf_target,
                "btcio writer configured",
            );
        }
    }
}

#[cfg(all(test, feature = "sequencer"))]
mod resolve_writer_config_tests {
    use super::*;

    fn args(
        policy: BtcioFeePolicyArg,
        fee_rate: Option<u64>,
        mempool_url: Option<&str>,
    ) -> AdditionalConfig {
        let argv = ["alpen-client", "--sequencer-pubkey", &"0".repeat(64)];
        let mut cfg = <AdditionalConfig as clap::Parser>::parse_from(argv);
        cfg.btcio_fee_policy = policy;
        cfg.btcio_fee_rate = fee_rate;
        cfg.btcio_mempool_base_url = mempool_url.map(str::to_owned);
        cfg
    }

    #[test]
    fn fixed_requires_fee_rate() {
        let err = resolve_writer_config(&args(BtcioFeePolicyArg::Fixed, None, None)).unwrap_err();
        assert!(err.to_string().contains("--btcio-fee-rate"));
    }

    #[test]
    fn fixed_one_sat_vb() {
        let cfg = resolve_writer_config(&args(BtcioFeePolicyArg::Fixed, Some(1), None)).unwrap();
        assert_eq!(cfg.fee_policy(), &FeePolicy::Fixed { fee_rate: 1 });
    }

    #[test]
    fn mempool_requires_base_url() {
        let err = resolve_writer_config(&args(BtcioFeePolicyArg::Mempool, None, None)).unwrap_err();
        assert!(err.to_string().contains("--btcio-mempool-base-url"));
    }

    #[test]
    fn mempool_rejects_empty_base_url() {
        let err =
            resolve_writer_config(&args(BtcioFeePolicyArg::Mempool, None, Some(""))).unwrap_err();
        assert!(err.to_string().contains("--btcio-mempool-base-url"));
    }

    #[test]
    fn mempool_with_url_succeeds() {
        let cfg = resolve_writer_config(&args(
            BtcioFeePolicyArg::Mempool,
            None,
            Some("https://mempool.space/signet"),
        ))
        .unwrap();
        match cfg.fee_policy() {
            FeePolicy::MempoolExplorer {
                mempool_base_url, ..
            } => assert_eq!(mempool_base_url, "https://mempool.space/signet"),
            other => panic!("expected MempoolExplorer, got {other:?}"),
        }
    }

    #[test]
    fn bitcoind_uses_conf_target() {
        let mut a = args(BtcioFeePolicyArg::Bitcoind, None, None);
        a.btcio_conf_target = 4;
        let cfg = resolve_writer_config(&a).unwrap();
        assert_eq!(cfg.fee_policy(), &FeePolicy::BitcoinD { conf_target: 4 });
    }
}

/// Parse the EE block time from the environment variable.
#[cfg(feature = "sequencer")]
fn block_builder_config_from_env(sequencer_enabled: bool) -> eyre::Result<BlockBuilderConfig> {
    let default_config = BlockBuilderConfig::default();
    if !sequencer_enabled {
        return Ok(default_config);
    }

    let blocktime_ms = match env::var(ALPEN_EE_BLOCK_TIME_MS_ENV_VAR) {
        Ok(raw_value) => {
            let blocktime_ms = raw_value.parse::<u64>().wrap_err_with(|| {
                format!(
                    "Failed to parse {ALPEN_EE_BLOCK_TIME_MS_ENV_VAR} as a positive integer milliseconds value: {raw_value}"
                )
            })?;
            if blocktime_ms == 0 {
                eyre::bail!("{ALPEN_EE_BLOCK_TIME_MS_ENV_VAR} must be greater than zero");
            }
            info!(
                blocktime_ms,
                env_var = ALPEN_EE_BLOCK_TIME_MS_ENV_VAR,
                "Using EE block time override from environment"
            );
            blocktime_ms
        }
        Err(env::VarError::NotPresent) => {
            let default_blocktime_ms = default_config.blocktime_ms();
            info!(
                blocktime_ms = default_blocktime_ms,
                "Using default EE block time"
            );
            return Ok(default_config);
        }
        Err(env::VarError::NotUnicode(_)) => {
            eyre::bail!("{ALPEN_EE_BLOCK_TIME_MS_ENV_VAR} must contain valid unicode");
        }
    };

    Ok(default_config.with_blocktime_ms(blocktime_ms))
}

/// Handle genesis related tasks.
/// Mainly deals with ensuring database has minimal expected state.
async fn ensure_genesis<TStorage: Storage + ExecBlockStorage + BatchStorage>(
    config: &AlpenEeConfig,
    genesis_epoch: &EpochCommitment,
    storage: &TStorage,
) -> eyre::Result<()> {
    ensure_genesis_ee_account_state(config, genesis_epoch, storage).await?;
    #[cfg(feature = "sequencer")]
    ensure_finalized_exec_chain_genesis(config, genesis_epoch.to_block_commitment(), storage)
        .await?;
    #[cfg(feature = "sequencer")]
    ensure_batch_genesis(config, storage).await?;
    Ok(())
}
