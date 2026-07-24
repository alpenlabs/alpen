//! Reth node for the Alpen codebase.
//!
//! # Logging
//!
//! Alpen (non-reth) logs carry a `component = "alpen"` field so they can be
//! filtered apart from the embedded reth logs in monitoring. The field is
//! attached via `info_span!(..., component = "alpen")` spans, so it is only
//! present while those spans are enabled. Run this crate with the `alpen_client`
//! target at INFO or a more verbose level to get the tags: lowering it (e.g.
//! `RUST_LOG=alpen_client=warn`) or capping the compile-time level below info
//! (`tracing/release_max_level_*`) disables the spans and silently drops the tag.

mod dummy_ol_client;
#[cfg(feature = "sequencer")]
mod gas_data_provider;
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
use std::time::Duration;
use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
    sync::Arc,
};

use alpen_chainspec::{
    chain_value_parser, ee_genesis_block_info, AlpenChainSpecParser, AlpenEeGenesisBlockInfo,
};
use alpen_ee_common::{
    chain_status_checked, BatchStorage, BlockNumHash, ChunkStorage, ExecBlockStorage, OLClient,
    SequencerOLClient, Storage,
};
use alpen_ee_config::{AlpenEeConfig, AlpenEeParams};
use alpen_ee_database::init_db_storage;
use alpen_ee_engine::{create_engine_control_task, sync_chainstate_to_engine, AlpenRethExecEngine};
#[cfg(feature = "sequencer")]
use alpen_ee_exec_chain::{init_exec_chain_state_from_storage, ExecChainState};
#[cfg(feature = "sequencer")]
use alpen_ee_genesis::ensure_finalized_exec_chain_genesis;
use alpen_ee_genesis::{ensure_batch_genesis, ensure_genesis_ee_account_state};
use alpen_ee_ol_tracker::init_ol_tracker_state;
use alpen_ee_rpc_server::{AlpenEeRpcServer, EeRpcContext, EeRpcServer, StaticFeeModelConfig};
#[cfg(feature = "sequencer")]
use alpen_ee_sequencer::{
    block_builder_task, build_ol_chain_tracker, init_batch_builder_state, init_lifecycle_state,
    init_ol_chain_tracker_state, sealing_policy::block_count_policy::BlockCountPolicy,
    BatchBuilderState, BatchLifecycleState, BlockBuilderConfig, OLChainTrackerState,
};
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
use reth_provider::{BlockNumReader, CanonStateSubscriptions};
#[cfg(feature = "sequencer")]
use strata_btcio::{
    broadcaster::BroadcasterBuilder, writer::chunked_envelope::create_chunked_envelope_task,
    BtcioParams,
};
use strata_common::healthz::{start_health_check_server, HealthCheckState};
#[cfg(feature = "sequencer")]
use strata_config::btcio::{
    fee_rate_from_sat_per_vb, fee_rate_to_sat_per_vb, FeePolicy, L1FeePolicyConfig,
    MempoolExplorerFeePolicy, WriterConfig,
};
use strata_identifiers::{EpochCommitment, OLBlockId};
use strata_l1_txfmt::MagicBytes;
use strata_logging::{init_logging_from_config, LoggingInitConfig};
use strata_predicate::PredicateKey;
use strata_primitives::{buf::Buf32, L1Height};
#[cfg(not(feature = "sp1"))]
use strata_zkvm_hosts as _;
use tokio::{
    runtime::Handle,
    sync::{mpsc, watch},
};
use tracing::{error, info, info_span, warn, Instrument};

#[cfg(feature = "sequencer")]
mod sequencer_imports {
    pub(super) use alloy_primitives::{address, Address};
    pub(super) use alpen_ee_da_provider::{
        ChunkedEnvelopeDaProvider, DaBlobSource, StateDiffBlobProvider,
    };
    pub(super) use alpen_reth_witness::RangeWitnessExtractor;
    pub(super) use strata_paas::{
        Prover, ProverBuilder, ProverHandle, ProverServiceBuilder, ReceiptStore, RetryConfig,
        TaskStore,
    };
    pub(super) use strata_proofimpl_alpen_acct::EeAcctProgram;
    pub(super) use strata_proofimpl_alpen_chunk::EeChunkProgram;
    pub(super) use strata_proofimpl_predicate_keys::{
        validate_expected_predicate_key, NativeAlpenAcctPredicateKey, NativeAlpenChunkPredicateKey,
        PredicateKeyProvider, Sp1Groth16PredicateKey,
    };
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

    pub(super) const DEFAULT_BENEFICIARY_ADDRESS: Address =
        address!("5400000000000000000000000000000000000010");
}

#[cfg(feature = "sequencer")]
use sequencer_imports::*;

use crate::{
    dummy_ol_client::DummyOLClient,
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
const DEFAULT_PROVER_FEE_PER_GAS_WEI: u64 = 15;
const DEFAULT_DA_OVERHEAD_MULTIPLIER_BPS: u32 = 10_000;
const DEFAULT_OL_OVERHEAD_WEI: u64 = 0;

/// Default end-to-end deadline applied to the SP1 prover network for the EE
/// chunk + acct provers when `--sp1-proof-deadline-secs` is not set. Chosen
/// to comfortably cover chunk/acct proofs while still failing fast on stuck
/// requests.
#[cfg(all(feature = "sequencer", feature = "sp1"))]
const DEFAULT_SP1_DEADLINE_SECS: u64 = 4 * 60 * 60;

/// Default capacity for the batch builder → chunk builder event channel.
#[cfg(feature = "sequencer")]
const DEFAULT_BATCH_EVENT_CHANNEL_CAPACITY: usize = 64;

/// Startup state that only the EE sequencer needs.
///
/// Bundled into one value so it can be gated behind a single runtime
/// `--sequencer` check and carried as a single `Option`.
#[cfg(feature = "sequencer")]
struct SequencerBootState {
    ol_chain_tracker: OLChainTrackerState,
    exec_chain: ExecChainState,
    batch_builder: BatchBuilderState<BlockCountPolicy>,
    batch_lifecycle: BatchLifecycleState,
}

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
                    .instrument(info_span!("start_health_check_server", component = "alpen"))
                    .await
                    .context("failed to start health check server")?;
            info!(target: "alpen-client", component = "alpen", %health_check_addr, "health check server started");

            // --- CONFIGS ---
            let datadir = builder.config().datadir().data_dir().to_path_buf();

            // TODO(STR-2982): read config, params from file
            let genesis_info = ee_genesis_block_info(&ext.custom_chain);

            info!(target: "alpen-client", component = "alpen", blockhash=%genesis_info.blockhash(), "EE genesis info");
            let params = load_ee_params(&ext.ee_params)?;
            validate_ee_params_genesis(&params, &genesis_info)?;
            let bridge_params = *params.bridge_params();
            info!(target: "alpen-client", component = "alpen", ?params, sequencer = ext.sequencer, "Starting EE Node");

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

            let db_handle = Handle::current();
            let storage: Arc<_> = dbs.node_storage(db_handle.clone()).into();

            let ol_client = if ext.dummy_ol_client {
                use strata_identifiers::Buf32;
                use strata_primitives::EpochCommitment;
                let genesis_epoch = EpochCommitment::new(0, 0, OLBlockId::from(Buf32([1; 32])));
                info!(target: "alpen-client", component = "alpen", "Using dummy OL client (no real OL connection)");
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
                .instrument(info_span!("account_genesis_epoch", component = "alpen"))
                .await
                .context("failed to fetch account genesis epoch from OL")?;

            ensure_genesis(
                config.as_ref(),
                &genesis_epoch,
                storage.as_ref(),
                ext.sequencer,
            )
            .instrument(info_span!("ensure_genesis", component = "alpen"))
            .await
            .context("genesis should not fail")?;

            let ol_chain_status = chain_status_checked(ol_client.as_ref())
                .instrument(info_span!("chain_status_check", component = "alpen"))
                .await
                .context("cannot fetch OL chain status")?;

            let ol_tracker_state = init_ol_tracker_state(ol_chain_status, storage.as_ref())
                .instrument(info_span!("init_ol_tracker", component = "alpen"))
                .await
                .context("ol tracker state initialization should not fail")?;

            // Sequencer-only startup state. Gated on the runtime `--sequencer` flag.
            #[cfg(feature = "sequencer")]
            let sequencer_boot_state = if ext.sequencer {
                let ol_chain_tracker =
                    init_ol_chain_tracker_state(storage.as_ref(), ol_client.as_ref())
                        .instrument(info_span!("init_ol_chain_tracker", component = "alpen"))
                        .await
                        .context("ol chain tracker state initialization should not fail")?;
                let exec_chain = init_exec_chain_state_from_storage(storage.as_ref())
                    .instrument(info_span!("init_exec_chain", component = "alpen"))
                    .await
                    .context("exec chain state initialization should not fail")?;
                let batch_builder = init_batch_builder_state(storage.as_ref())
                    .instrument(info_span!("init_batch_builder", component = "alpen"))
                    .await
                    .context("batch builder state initialization should not fail")?;
                let batch_lifecycle = init_lifecycle_state(storage.as_ref())
                    .instrument(info_span!("init_lifecycle", component = "alpen"))
                    .await
                    .context("batch lifecycle state initialization should not fail")?;
                Some(SequencerBootState {
                    ol_chain_tracker,
                    exec_chain,
                    batch_builder,
                    batch_lifecycle,
                })
            } else {
                None
            };

            let initial_preconf_head = {
                #[cfg(feature = "sequencer")]
                {
                    if let Some(boot) = sequencer_boot_state.as_ref() {
                        boot.exec_chain.tip_blocknumhash()
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
            // --- INITIALIZE SERVICES ---

            // Create gossip channel before building the node so we can register it early
            let (gossip_tx, gossip_rx) = mpsc::unbounded_channel();

            // Create preconf channel for p2p head block gossip -> engine control integration
            // This channel sends block hash and number received from peers to the engine control
            // task
            let (preconf_tx, preconf_rx) = watch::channel(initial_preconf_head);
            let initial_fee_config = ext.sequencer.then(|| fee_config_from_ext(&ext));
            let (fee_config_tx, fee_config_rx) = watch::channel(initial_fee_config);

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

            let evm_factory = AlpenEvmFactory::from_bridge_params(&bridge_params);
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
                            info!(target: "alpen-gossip", component = "alpen", "Registered Alpen gossip RLPx subprotocol");
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
                info!(target: "alpen-client", component = "alpen", "installed StateDiffGenerator exex for DA");

                // Per-block accessed-state capture. The CHUNK proof's witness is
                // now produced inline during payload build (see the EE node's
                // `try_build_payload` / `AlpenRethPayloadEngine`); this exex
                // remains only to feed the ACCOUNT proof's batch-range witness
                // (`RangeWitnessExtractor` reads `AccessedStateStore`). Retiring
                // it is a separate acct-proof migration tracked as follow-up
                // work to STR-3649.
                node_builder = node_builder.install_exex("accessed_state", {
                    let accessed_state_store = storage.clone();
                    |ctx| async {
                        Ok(AccessedStateGenerator::new(ctx, accessed_state_store).start())
                    }
                });
                info!(target: "alpen-client", component = "alpen", "installed AccessedStateGenerator exex (account-proof range witness)");
            }

            node_builder = node_builder.extend_rpc_modules({
                let consensus_watcher = consensus_watcher.clone();
                let fee_config_rx = fee_config_rx.clone();
                let storage = storage.clone();
                move |ctx| {
                    let provider = ctx.provider().clone();
                    let rpc_context =
                        EeRpcContext::new(storage.clone(), storage.clone(), fee_config_rx.clone());
                    let ee_rpc_server = EeRpcServer::new(provider, consensus_watcher, rpc_context);
                    ctx.modules.merge_configured(ee_rpc_server.into_rpc())?;
                    Ok(())
                }
            });

            let handle = node_builder.launch().await?;

            let node = &handle.node;

            // Sync chainstate to engine for sequencer nodes before starting other tasks
            #[cfg(feature = "sequencer")]
            if ext.sequencer {
                let engine = AlpenRethExecEngine::new(node.beacon_engine_handle.clone());
                let storage_clone = storage.clone();
                let provider_clone = node.provider.clone();

                // Block on the async sync operation
                let sync_result =
                    sync_chainstate_to_engine(storage_clone.as_ref(), &provider_clone, &engine)
                        .instrument(info_span!("chainstate_sync", component = "alpen"))
                        .await;

                if let Err(e) = sync_result {
                    error!(target: "alpen-client", component = "alpen", error = ?e, "failed to sync chainstate to engine on startup");
                    return Err(eyre::eyre!("chainstate sync failed: {e}"));
                }

                info!(target: "alpen-client", component = "alpen", "chainstate sync completed successfully");
            }

            let engine_control_task = create_engine_control_task(
                preconf_rx.clone(),
                consensus_watcher.clone(),
                node.provider.clone(),
                AlpenRethExecEngine::new(node.beacon_engine_handle.clone()),
            );

            // Subscribe to canonical state notifications for broadcasting new blocks
            let state_events = node.provider.subscribe_to_canonical_state();

            let initial_highest_seq_no = match node.provider.best_block_number() {
                Ok(block_number) => block_number,
                Err(err) => {
                    warn!(
                        target: "alpen-gossip",
                        component = "alpen",
                        ?err,
                        "failed to read canonical head for gossip replay guard; falling back to zero"
                    );
                    0
                }
            };

            // Create gossip task for broadcasting new blocks
            let gossip_task = create_gossip_task(
                gossip_rx,
                state_events,
                preconf_tx.clone(),
                fee_config_tx,
                fee_config_rx.clone(),
                initial_highest_seq_no,
                gossip_config,
            );

            // Spawn critical tasks
            node.task_executor.spawn_critical(
                "engine_control",
                engine_control_task.instrument(info_span!("engine_control", component = "alpen")),
            );
            node.task_executor.spawn_critical(
                "gossip_task",
                gossip_task.instrument(info_span!("gossip_task", component = "alpen")),
            );

            #[cfg(feature = "sequencer")]
            if let Some(SequencerBootState {
                ol_chain_tracker: ol_chain_tracker_state,
                exec_chain: exec_chain_state,
                batch_builder: batch_builder_state,
                batch_lifecycle: batch_lifecycle_state,
            }) = sequencer_boot_state
            {
                // sequencer specific tasks

                use alpen_ee_common::{require_latest_batch, BlockNumHash};
                use alpen_ee_sequencer::{
                    create_batch_builder, create_batch_lifecycle_task,
                    create_update_submitter_task,
                    sealing_policy::{
                        block_count_policy::{BlockCountDataProvider, FixedBlockCountSealing},
                        gas_limit_policy::MaxGasSealing,
                        or_policy::OrSealing,
                    },
                    BatchBuilderEvent,
                };

                use crate::gas_data_provider::RethGasDataProvider;

                let payload_engine = Arc::new(AlpenRethPayloadEngine::new(
                    node.payload_builder_handle.clone(),
                    node.beacon_engine_handle.clone(),
                    ext.beneficiary_address,
                    storage.clone(),
                ));

                let exec_chain_handle = services::exec_chain::start_exec_chain_service(
                    exec_chain_state,
                    preconf_tx.clone(),
                    storage.clone(),
                    consensus_watcher.clone(),
                    &service_executor,
                )
                .instrument(info_span!("start_exec_chain", component = "alpen"))
                .await
                .map_err(|e| eyre::eyre!("failed to start exec chain service: {e}"))?;

                let (ol_chain_tracker, ol_chain_tracker_task) = build_ol_chain_tracker(
                    ol_chain_tracker_state,
                    status_watcher.clone(),
                    ol_client.clone(),
                    storage.clone(),
                );

                let (latest_batch, _) = require_latest_batch(storage.as_ref())
                    .instrument(info_span!("require_latest_batch", component = "alpen"))
                    .await?;

                let batch_sealing_policy =
                    FixedBlockCountSealing::new(ext.batch_sealing_block_count);
                let block_data_provider = Arc::new(BlockCountDataProvider);

                // Per-block proof witnesses are captured inline during payload
                // build and persisted by `AlpenRethPayloadEngine`, and the
                // chunk prover's `ChunkSpec::fetch_input` assembles a chunk
                // proof input from those per-block records. There is no
                // chunk-seal extraction step and no chunk-spanning multiproof.

                // Channel from batch builder → chunk builder.
                let (batch_event_tx, batch_event_rx) = mpsc::channel::<BatchBuilderEvent>(
                    ext.batch_event_channel_capacity
                        .unwrap_or(DEFAULT_BATCH_EVENT_CHANNEL_CAPACITY),
                );

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
                    Some(batch_event_tx),
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
                    target: "alpen-client", component = "alpen",
                    retry_count = ext.btcio_retry_count,
                    retry_interval_ms = ext.btcio_retry_interval,
                    "btcio Bitcoin RPC retry policy configured",
                );

                // Sequencer address from bitcoin wallet.
                let sequencer_address = btc_client
                    .get_new_address()
                    .await
                    .map_err(|e| eyre::eyre!("failed to get sequencer address: {e}"))?;

                // Wrap raw DBs in ops using the shared runtime handle.
                let broadcast_ops = Arc::new(dbs.broadcast_ops(db_handle.clone()));
                let envelope_ops = Arc::new(dbs.chunked_envelope_ops(db_handle));

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

                let blob_provider: Arc<dyn DaBlobSource> = Arc::new(StateDiffBlobProvider::new(
                    storage.clone(),
                    dbs.witness_db(),
                    header_summary,
                    dbs.da_context_db(),
                ));

                let batch_da_provider = Arc::new(ChunkedEnvelopeDaProvider::new(
                    blob_provider,
                    envelope_handle,
                    broadcast_ops,
                    btc_client.clone(),
                    magic_bytes,
                )?);

                // Spawn btcio tasks.
                node.task_executor.spawn_critical(
                    "chunked_envelope_watcher",
                    envelope_watcher_task
                        .instrument(info_span!("chunked_envelope_watcher", component = "alpen")),
                );

                info!(target: "alpen-client", component = "alpen", "btcio DA pipeline started");

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
                let chunk_storage_dyn: Arc<dyn ChunkStorage> = storage.clone();

                let genesis = {
                    use alpen_reth_exex::alloy2reth::IntoRspChainConfig as _;
                    ext.custom_chain.genesis().config.clone().into_rsp()
                };

                let chunk_builder = ProverBuilder::new(ChunkSpec::new(
                    chunk_storage_dyn.clone(),
                    storage.clone(),
                    genesis.clone(),
                    bridge_params,
                ))
                .task_store(task_store.clone())
                .receipt_store(chunk_receipts.clone())
                .receipt_hook(ChunkReceiptHook::new(chunk_storage_dyn.clone()))
                .retry(RetryConfig::default());

                // NOTE: the account prover still assembles its batch-range
                // witness via `RangeWitnessExtractor`, which reads the
                // per-block accessed-state records the (now removed)
                // `AccessedStateGenerator` exex used to write. Migrating this to
                // the inline per-block witnesses is the remaining step to fully
                // retire the exex + the deep range multiproof (see
                // experimental/evgeniy/ee-proper-witness.md).
                let range_witness_extractor = Arc::new(RangeWitnessExtractor::new(
                    node.provider.clone(),
                    storage.clone(),
                ));
                let acct_range_witness_fn: Arc<AcctRangeWitnessFn> = {
                    let extractor = range_witness_extractor.clone();
                    Arc::new(move |first_block, last_block| {
                        extractor.extract_range_witness(first_block, last_block)
                    })
                };

                let acct_builder = ProverBuilder::new(AcctSpec::new(
                    chunk_receipts.clone(),
                    batch_storage_dyn.clone(),
                    chunk_storage_dyn.clone(),
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

                let batch_prover = launch_validated_ee_batch_prover(
                    ol_client.as_ref(),
                    &service_executor,
                    EeProverBuilders {
                        chunk: chunk_builder,
                        account: acct_builder,
                    },
                    EeProverStores {
                        chunk_storage: chunk_storage_dyn,
                        batch_proofs,
                    },
                    ext.dev_native_prover,
                    ext.sp1_proof_deadline_secs,
                )
                .await?;

                info!(
                    target: "alpen-client",
                    component = "alpen",
                    "EE chunk + acct paas provers started"
                );

                let (batch_lifecycle_handle, batch_lifecycle_task) = create_batch_lifecycle_task(
                    None,
                    batch_lifecycle_state,
                    batch_builder_handle.latest_batch_watcher(),
                    batch_da_provider,
                    batch_prover.clone(),
                    storage.clone(),
                );

                let update_submitter_task = create_update_submitter_task(
                    ol_client,
                    storage.clone(),
                    storage.clone(),
                    batch_prover,
                    batch_lifecycle_handle.latest_proof_ready_watcher(),
                    status_watcher,
                );

                node.task_executor.spawn_critical(
                    "ol_chain_tracker",
                    ol_chain_tracker_task
                        .instrument(info_span!("ol_chain_tracker", component = "alpen")),
                );
                // Per-block proof witnesses are captured inline during payload
                // build (in the EE node's `try_build_payload`) and persisted by
                // the payload engine (`AlpenRethPayloadEngine`) before the
                // payload is returned, so the block builder runs no separate
                // witness step. The chunk prover's `ChunkSpec::fetch_input`
                // assembles a chunk proof input from those per-block records.
                node.task_executor.spawn_critical(
                    "block_assembly",
                    block_builder_task(
                        block_builder_config,
                        exec_chain_handle,
                        ol_chain_tracker,
                        payload_engine,
                        storage.clone(),
                    )
                    .instrument(info_span!("block_assembly", component = "alpen")),
                );

                // --- Chunk builder service ---
                let chunk_block_count = ext
                    .chunk_sealing_block_count
                    .unwrap_or(ext.batch_sealing_block_count);
                let genesis_blocknumhash =
                    BlockNumHash::new(genesis_info.blockhash().0.into(), genesis_info.blocknum());

                // Validate --chunk-sealing-gas-limit if configured.
                //
                // EIP-1559 lets the per-block gas limit drift from genesis by
                // ±1/1024 per block, so the actual block gas limit at runtime
                // may be slightly higher than genesis. We use 2× the genesis
                // gas limit as a conservative floor to accommodate this drift
                // while still catching obvious misconfigurations.
                if let Some(configured) = ext.chunk_sealing_gas_limit {
                    let min_chunk_gas = ext.custom_chain.genesis().gas_limit.saturating_mul(2);
                    eyre::ensure!(
                        configured >= min_chunk_gas,
                        "--chunk-sealing-gas-limit ({configured}) is below the minimum \
                         ({min_chunk_gas}, 2× genesis block gas limit {}). A single block \
                         can use up to the per-block gas limit, so the chunk budget must \
                         be large enough to always fit at least one block.",
                        ext.custom_chain.genesis().gas_limit,
                    );
                }

                // u64::MAX effectively disables the gas policy while keeping a
                // single monomorphic code path (no dyn / enum branching).
                let chunk_gas_limit = ext.chunk_sealing_gas_limit.unwrap_or(u64::MAX);
                let chunk_sealing_policy = OrSealing::new(
                    FixedBlockCountSealing::new(chunk_block_count),
                    MaxGasSealing::new(chunk_gas_limit),
                );

                services::chunk_builder::start_chunk_builder_service(
                    genesis_blocknumhash,
                    storage.clone(),
                    storage.clone(),
                    storage.clone(),
                    chunk_sealing_policy,
                    RethGasDataProvider::new(node.provider.clone()),
                    batch_event_rx,
                    &service_executor,
                )
                .await
                .map_err(|e| eyre::eyre!("failed to launch chunk builder service: {e}"))?;

                node.task_executor.spawn_critical(
                    "ee_batch_builder",
                    batch_builder_task
                        .instrument(info_span!("ee_batch_builder", component = "alpen")),
                );
                node.task_executor.spawn_critical(
                    "ee_batch_lifecycle",
                    batch_lifecycle_task
                        .instrument(info_span!("ee_batch_lifecycle", component = "alpen")),
                );
                node.task_executor.spawn_critical(
                    "ee_update_submitter",
                    update_submitter_task
                        .instrument(info_span!("ee_update_submitter", component = "alpen")),
                );
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

    /// JSON-serialized Alpen EE chain params.
    #[arg(long, value_name = "PATH", required = true)]
    pub ee_params: PathBuf,

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

    /// Static proving fee charged per unit of raw EVM gas.
    #[arg(long, default_value_t = DEFAULT_PROVER_FEE_PER_GAS_WEI)]
    pub prover_fee_per_gas_wei: u64,

    /// Basis-points multiplier applied to estimated DA cost.
    #[arg(long, default_value_t = DEFAULT_DA_OVERHEAD_MULTIPLIER_BPS)]
    pub da_overhead_multiplier_bps: u32,

    /// Small additive fee charged for OL and infrastructure overhead.
    #[arg(long, default_value_t = DEFAULT_OL_OVERHEAD_WEI)]
    pub ol_overhead_wei: u64,

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

    /// Number of blocks per chunk before sealing.
    /// Defaults to `batch_sealing_block_count` if not set.
    #[arg(long, required = false)]
    pub chunk_sealing_block_count: Option<u64>,

    /// Cumulative gas limit per chunk before sealing.
    /// When set, a chunk is sealed when either the block count or the gas
    /// limit is reached (whichever comes first). When omitted, only the
    /// block count policy is used.
    #[arg(long, required = false)]
    pub chunk_sealing_gas_limit: Option<u64>,

    /// Capacity of the batch builder → chunk builder event channel.
    /// Defaults to 64 if not set.
    #[cfg(feature = "sequencer")]
    #[arg(long, required = false)]
    pub batch_event_channel_capacity: Option<usize>,

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
    pub btcio_fee_rate: Option<f64>,

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

    #[cfg(feature = "sequencer")]
    #[arg(long, default_value_t = DEFAULT_BENEFICIARY_ADDRESS)]
    pub beneficiary_address: Address,
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

/// Loads Alpen EE chain params from a JSON file.
fn load_ee_params(path: &Path) -> eyre::Result<AlpenEeParams> {
    let json = fs::read_to_string(path)
        .with_context(|| format!("failed to read EE params file {path:?}"))?;
    AlpenEeParams::from_json_str(&json)
        .with_context(|| format!("failed to parse EE params file {path:?}"))
}

/// Validates that EE params describe the selected execution genesis block.
fn validate_ee_params_genesis(
    params: &AlpenEeParams,
    genesis_info: &AlpenEeGenesisBlockInfo,
) -> eyre::Result<()> {
    if params.genesis_blockhash() != genesis_info.blockhash() {
        eyre::bail!(
            "EE params genesis blockhash {} does not match chain genesis blockhash {}",
            params.genesis_blockhash(),
            genesis_info.blockhash()
        );
    }

    if params.genesis_stateroot() != genesis_info.stateroot() {
        eyre::bail!(
            "EE params genesis stateroot {} does not match chain genesis stateroot {}",
            params.genesis_stateroot(),
            genesis_info.stateroot()
        );
    }

    if params.genesis_blocknum() != genesis_info.blocknum() {
        eyre::bail!(
            "EE params genesis block number {} does not match chain genesis block number {}",
            params.genesis_blocknum(),
            genesis_info.blocknum()
        );
    }

    Ok(())
}

#[cfg(feature = "sequencer")]
async fn launch_validated_ee_batch_prover(
    ol_client: &(impl SequencerOLClient + Send + Sync),
    service_executor: &ServiceExecutor,
    builders: EeProverBuilders,
    stores: EeProverStores,
    use_native_prover: bool,
    sp1_deadline_secs: Option<u64>,
) -> eyre::Result<Arc<PaasBatchProver>> {
    let ol_account_update_vk = ol_client
        .get_latest_account_update_vk()
        .await
        .context("failed to fetch OL account update_vk for prover validation")?;
    let backend = if use_native_prover {
        EeProverBackend::Native
    } else {
        EeProverBackend::Sp1 {
            deadline_secs: sp1_deadline_secs,
        }
    };
    let prover_config = build_ee_prover_config(builders, backend).await?;

    validate_ee_account_prover_predicate_key(
        &ol_account_update_vk,
        &prover_config.account_predicate_key,
    )?;

    let (chunk_handle, acct_handle) =
        launch_ee_prover_services(service_executor, prover_config.provers).await?;

    Ok(Arc::new(PaasBatchProver::new(
        chunk_handle,
        acct_handle,
        stores.chunk_storage,
        stores.batch_proofs,
    )))
}

#[cfg(feature = "sequencer")]
struct EeProverBuilders {
    chunk: ProverBuilder<ChunkSpec>,
    account: ProverBuilder<AcctSpec>,
}

#[cfg(feature = "sequencer")]
struct EeProverStores {
    chunk_storage: Arc<dyn ChunkStorage>,
    batch_proofs: Arc<EeBatchProofDbManager>,
}

#[cfg(feature = "sequencer")]
struct EeProverConfig {
    provers: EeProvers,
    account_predicate_key: PredicateKey,
}

#[cfg(feature = "sequencer")]
struct EeProvers {
    chunk: Prover<ChunkSpec>,
    account: Prover<AcctSpec>,
}

#[cfg(feature = "sequencer")]
enum EeProverBackend {
    Native,
    Sp1 { deadline_secs: Option<u64> },
}

#[cfg(feature = "sequencer")]
async fn build_ee_prover_config(
    builders: EeProverBuilders,
    backend: EeProverBackend,
) -> eyre::Result<EeProverConfig> {
    match backend {
        EeProverBackend::Native => {
            info!(
                target: "alpen-client",
                "EE chunk + acct provers: native host (dev/test only)"
            );

            let chunk = builders.chunk.native(EeChunkProgram::native_host());
            let chunk_predicate_key = NativeAlpenChunkPredicateKey
                .predicate_key()
                .expect("native chunk predicate key must be available");
            let acct_program = EeAcctProgram::new(chunk_predicate_key);
            let account = builders.account.native(acct_program.native_host());
            let account_predicate_key = NativeAlpenAcctPredicateKey
                .predicate_key()
                .expect("native account predicate key must be available");

            Ok(EeProverConfig {
                provers: EeProvers { chunk, account },
                account_predicate_key,
            })
        }
        #[cfg(feature = "sp1")]
        EeProverBackend::Sp1 { deadline_secs } => {
            use zkaleido::ZkVmExecutor;

            let deadline_secs = deadline_secs.unwrap_or(DEFAULT_SP1_DEADLINE_SECS);
            let deadline = Duration::from_secs(deadline_secs);
            info!(
                target: "alpen-client",
                deadline_secs,
                "sp1 EE prover deadline configured"
            );

            let sp1_config = SP1HostConfig::default().with_deadline(deadline);
            let chunk_host: SP1Host = (**alpen_chunk_host(sp1_config.clone()).await).clone();
            let acct_host: SP1Host = (**alpen_acct_host(sp1_config).await).clone();
            let account_predicate_key = Sp1Groth16PredicateKey::new(acct_host.program_id().0)
                .predicate_key()
                .map_err(|e| {
                    eyre::eyre!("failed to derive local SP1 account prover predicate key: {e}")
                })?;

            Ok(EeProverConfig {
                provers: EeProvers {
                    chunk: builders.chunk.remote(chunk_host),
                    account: builders.account.remote(acct_host),
                },
                account_predicate_key,
            })
        }
        #[cfg(not(feature = "sp1"))]
        EeProverBackend::Sp1 { .. } => Err(eyre::eyre!(
            "remote SP1 prover is not compiled in; pass --dev-native-prover \
             or build with the `sp1` feature"
        )),
    }
}

#[cfg(feature = "sequencer")]
async fn launch_ee_prover_services(
    service_executor: &ServiceExecutor,
    provers: EeProvers,
) -> eyre::Result<(ProverHandle<ChunkSpec>, ProverHandle<AcctSpec>)> {
    let prover_tick = Duration::from_secs(5);
    let chunk_handle = ProverServiceBuilder::new(provers.chunk)
        .tick_interval(prover_tick)
        .launch(service_executor)
        .await
        .map_err(|e| eyre::eyre!("launching chunk prover service: {e}"))?;
    let acct_handle = ProverServiceBuilder::new(provers.account)
        .tick_interval(prover_tick)
        .launch(service_executor)
        .await
        .map_err(|e| eyre::eyre!("launching acct prover service: {e}"))?;

    Ok((chunk_handle, acct_handle))
}

#[cfg(feature = "sequencer")]
fn validate_ee_account_prover_predicate_key(
    ol_update_vk: &PredicateKey,
    local_predicate_key: &PredicateKey,
) -> eyre::Result<()> {
    validate_expected_predicate_key(ol_update_vk, local_predicate_key).map_err(|e| {
        eyre::eyre!(
            "OL account update_vk does not match local EE account prover predicate key: {e}"
        )
    })
}

#[cfg(test)]
mod additional_config_tests {
    use strata_predicate::PredicateTypeId;

    use super::*;

    #[cfg(feature = "sequencer")]
    #[test]
    fn ee_account_prover_predicate_key_validation_accepts_match() {
        let predicate = PredicateKey::new(PredicateTypeId::Bip340Schnorr, vec![1, 2, 3]);

        validate_ee_account_prover_predicate_key(&predicate, &predicate).unwrap();
    }

    #[cfg(feature = "sequencer")]
    #[test]
    fn ee_account_prover_predicate_key_validation_rejects_mismatch() {
        let ol_update_vk = PredicateKey::new(PredicateTypeId::Bip340Schnorr, vec![1, 2, 3]);
        let local_predicate_key = PredicateKey::new(PredicateTypeId::Sp1Groth16, vec![4, 5, 6]);

        let err = validate_ee_account_prover_predicate_key(&ol_update_vk, &local_predicate_key)
            .unwrap_err()
            .to_string();

        assert!(err.contains("OL account update_vk does not match local EE account prover"));
        assert!(err.contains("predicate key mismatch"));
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
            component = "alpen",
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

    info!(target: "alpen-client", component = "alpen", "logging initialized");

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

fn fee_config_from_ext(ext: &AdditionalConfig) -> StaticFeeModelConfig {
    // TODO(STR-2161): source these constants from the canonical SequencerFeeModelConfig once
    // quote() drives charging, so gossip/RPC match the values the sequencer actually charges.
    StaticFeeModelConfig::new(
        ext.prover_fee_per_gas_wei,
        ext.da_overhead_multiplier_bps,
        ext.ol_overhead_wei,
    )
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
            let fee_rate_sat_per_vb = ext.btcio_fee_rate.ok_or_else(|| {
                eyre::eyre!("--btcio-fee-rate is required when --btcio-fee-policy=fixed")
            })?;
            let fee_rate = fee_rate_from_sat_per_vb(fee_rate_sat_per_vb)
                .map_err(|err| eyre::eyre!("invalid --btcio-fee-rate: {err}"))?;
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
            info!(target: "alpen-client",
            component = "alpen",
            policy = "bitcoind",
            conf_target, "btcio writer configured",);
        }
        FeePolicy::Fixed { fee_rate } => {
            info!(
                target: "alpen-client",
                component = "alpen",
                policy = "fixed",
                fee_rate_sat_vb = fee_rate_to_sat_per_vb(*fee_rate),
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
                component = "alpen",
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
    use bitcoind_async_client::corepc_types::bitcoin::FeeRate;

    use super::*;

    fn args(
        policy: BtcioFeePolicyArg,
        fee_rate: Option<f64>,
        mempool_url: Option<&str>,
    ) -> AdditionalConfig {
        let argv = [
            "alpen-client",
            "--ee-params",
            "/tmp/ee-params.json",
            "--sequencer-pubkey",
            &"0".repeat(64),
        ];
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
        let cfg = resolve_writer_config(&args(BtcioFeePolicyArg::Fixed, Some(1.0), None)).unwrap();
        assert_eq!(
            cfg.fee_policy(),
            &FeePolicy::Fixed {
                fee_rate: FeeRate::from_sat_per_vb_u32(1)
            }
        );
    }

    #[test]
    fn fixed_half_sat_vb() {
        let cfg = resolve_writer_config(&args(BtcioFeePolicyArg::Fixed, Some(0.5), None)).unwrap();
        assert_eq!(
            cfg.fee_policy(),
            &FeePolicy::Fixed {
                fee_rate: FeeRate::from_sat_per_kwu(125)
            }
        );
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
                target: "alpen-client",
                component = "alpen",
                blocktime_ms,
                env_var = ALPEN_EE_BLOCK_TIME_MS_ENV_VAR,
                "Using EE block time override from environment"
            );
            blocktime_ms
        }
        Err(env::VarError::NotPresent) => {
            let default_blocktime_ms = default_config.blocktime_ms();
            info!(
                target: "alpen-client",
                component = "alpen",
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
    is_sequencer: bool,
) -> eyre::Result<()> {
    ensure_genesis_ee_account_state(config, genesis_epoch, storage).await?;

    #[cfg(feature = "sequencer")]
    if is_sequencer {
        ensure_finalized_exec_chain_genesis(config, genesis_epoch.to_block_commitment(), storage)
            .await?;
        ensure_batch_genesis(config, storage).await?;
    }
    #[cfg(not(feature = "sequencer"))]
    let _ = is_sequencer;

    Ok(())
}
