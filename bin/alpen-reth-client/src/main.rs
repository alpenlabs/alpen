//! Reth node for the Alpen codebase.

use std::{
    future::Future,
    os::macos::raw::stat,
    sync::Arc,
    time::{self, Duration},
};

use alloy_genesis::Genesis;
use alloy_primitives::{Address, FixedBytes, B256};
use alloy_rpc_types::engine::{ForkchoiceState, PayloadAttributes};
use alpen_chainspec::{chain_value_parser, AlpenChainSpecParser};
use alpen_reth_db::rocksdb::WitnessDB;
use alpen_reth_exex::{ProverWitnessGenerator, StateDiffGenerator};
use alpen_reth_node::{
    args::AlpenNodeArgs,
    payload::{AlpenBuiltPayload, AlpenPayloadBuilderAttributes},
    AlpenEngineTypes, AlpenEthereumNode, AlpenPayloadAttributes,
};
use alpen_reth_rpc::{AlpenRPC, StrataRpcApiServer};
use clap::Parser;
use futures::{
    stream::{self, Stream},
    StreamExt,
};
use reth_chain_state::CanonStateSubscriptions;
use reth_chainspec::ChainSpec;
use reth_cli_commands::{launcher::FnLauncher, node::NodeCommand};
use reth_cli_runner::CliRunner;
use reth_node_builder::{
    BuiltPayload, EngineApiMessageVersion, FullNodeComponents, NodeBuilder, NodeComponents,
    NodeHandle, NodeTypesWithDB, PayloadBuilderAttributes, PayloadTypes, WithLaunchContext,
};
use reth_node_core::{args::LogArgs, primitives::account};
use reth_provider::{
    providers::{BlockchainProvider, ProviderNodeTypes},
    BlockIdReader, BlockNumReader,
};
use tokio::{
    sync::{mpsc, Semaphore},
    time::MissedTickBehavior,
};
use tracing::{debug, error, info, warn};

use crate::{
    config::{Config, Params, SequencerConfig},
    engine::{
        AlpenRethExecEngine, ChainStateProvider, EnginePayloadAttributes, ExecutionEngine,
        PayloadBuilderEngine, RethChainStateProvider,
    },
    mock_client::get_mocked_client,
    ol_tracker::ol_tracker_stream,
    traits::{ELSequencerClient, L1Client, OlClient},
    types::{AccountId, AccountStateCommitment, ConsensusEvent, OlBlockId},
    utils::{ClockProvider, ExponentialBackoff, RetryTracker, SystemClock},
};

mod config;
mod engine;
mod errors;
mod mock_client;
mod ol_tracker;
mod rpc_client;
mod traits;
mod types;
mod utils;

fn main() {
    reth_cli_util::sigsegv_handler::install();

    // Enable backtraces unless a RUST_BACKTRACE value has already been explicitly provided.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        std::env::set_var("RUST_BACKTRACE", "1");
    }

    let mut command = NodeCommand::<AlpenChainSpecParser, AdditionalConfig>::parse();

    // use provided alpen chain spec
    command.chain = command.ext.custom_chain.clone();
    // disable peer discovery
    command.network.discovery.disable_discovery = true;
    // enable engine api v4
    command.engine.accept_execution_requests_hash = true;
    // allow chain fork blocks to be created
    command
        .engine
        .always_process_payload_attributes_on_canonical_head = true;

    command.rpc.http = true;

    let genesis = command.ext.custom_chain.genesis.clone();

    let config = Config {
        params: Params {
            account_id: FixedBytes::from([1u8; 32]).into(),
            genesis_config: genesis,
        },
        finality_depth: 2,
        sequencer_pubkey: FixedBytes::from([0u8; 32]),
    };

    let sequencer_config = Some(SequencerConfig {
        target_blocktime_ms: 5000,
        anchor_finality_depth: 1,
    });

    if let Err(err) = run(
        command,
        |builder: WithLaunchContext<NodeBuilder<Arc<reth_db::DatabaseEnv>, ChainSpec>>,
         ext: AdditionalConfig| async move {
            let datadir = builder.config().datadir().data_dir().to_path_buf();

            let node_args = AlpenNodeArgs {
                sequencer_http: ext.sequencer_http.clone(),
            };

            let mut node_builder = builder.node(AlpenEthereumNode::new(node_args));

            // let mut extend_rpc = None;

            // if ext.enable_witness_gen || ext.enable_state_diff_gen {
            //     let rbdb = db::open_rocksdb_database(datadir.clone()).expect("open rocksdb");
            //     let db = Arc::new(WitnessDB::new(rbdb));
            //     // Add RPC for querying block witness and state diffs.
            //     extend_rpc.replace(AlpenRPC::new(db.clone()));

            //     // Install Prover Input ExEx and persist to DB
            //     if ext.enable_witness_gen {
            //         let witness_db = db.clone();
            //         node_builder = node_builder.install_exex("prover_input", |ctx| async {
            //             Ok(ProverWitnessGenerator::new(ctx, witness_db).start())
            //         });
            //     }

            //     // Install State Diff ExEx and persist to DB
            //     if ext.enable_state_diff_gen {
            //         let state_diff_db = db.clone();
            //         node_builder = node_builder.install_exex("state_diffs", |ctx| async {
            //             Ok(StateDiffGenerator::new(ctx, state_diff_db).start())
            //         });
            //     }
            // }

            // Note: can only add single hook
            // node_builder = node_builder.extend_rpc_modules(|ctx| {
            //     if let Some(rpc) = extend_rpc {
            //         ctx.modules.merge_configured(rpc.into_rpc())?;
            //     }

            //     Ok(())
            // });

            node_builder = node_builder.on_node_started(|node| {
                let task_executor = node.task_executor.clone();

                let mocked_client = get_mocked_client();
                let reth_engine = AlpenRethExecEngine::new(
                    node.payload_builder_handle.clone(),
                    node.beacon_engine_handle.clone(),
                );

                let mut ol_events_stream = Box::pin(ol_tracker_stream(
                    config,
                    mocked_client.clone(),
                    mocked_client.clone(),
                ));

                let (consensus_evt_tx, mut consensus_evt_rx) = mpsc::channel::<ConsensusEvent>(64);

                if let Some(sequencer_config) = sequencer_config {
                    let chainstate_provider = RethChainStateProvider {
                        canonical_in_memory_state: node.provider.canonical_in_memory_state(),
                    };
                    task_executor.spawn_critical(
                        "block_assembly",
                        block_producer_worker(
                            sequencer_config.clone(),
                            chainstate_provider,
                            SystemClock,
                            reth_engine.clone(),
                            consensus_evt_tx.clone(),
                        ),
                    );

                    // spawn batch prover worker
                }

                let consensus_evt_rx = {
                    let (tx, rx) = mpsc::channel(64);

                    task_executor.spawn_critical("tap_mock_client", async move {
                        while let Some(ev) = consensus_evt_rx.recv().await {
                            let _ = tx.send(ev.clone()).await;

                            match ev {
                                ConsensusEvent::Head(latest_state_commitment) => {
                                    mocked_client
                                        .set_latest_account_commitment(latest_state_commitment)
                                        .await;
                                }
                                _ => {}
                            }
                        }
                    });

                    rx
                };

                task_executor.spawn_critical("consensus_events_source", async move {
                    while let Some(ev) = ol_events_stream.next().await {
                        consensus_evt_tx.send(ev).await.unwrap();
                    }
                });

                task_executor.spawn_critical(
                    "apply_consensus_events",
                    blockhash_sync_worker(node.provider.clone(), consensus_evt_rx, reth_engine),
                );

                Ok(())
            });

            let NodeHandle {
                node_exit_future, ..
            } = node_builder.launch().await?;

            node_exit_future.await
        },
    ) {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}

/// Our custom cli args extension that adds one flag to reth default CLI.
#[derive(Debug, clap::Parser)]
pub struct AdditionalConfig {
    #[command(flatten)]
    pub logs: LogArgs,

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

    #[arg(long, default_value_t = false)]
    pub enable_witness_gen: bool,

    #[arg(long, default_value_t = false)]
    pub enable_state_diff_gen: bool,

    /// Rpc of sequener's reth node to forward transactions to.
    #[arg(long, required = false)]
    pub sequencer_http: Option<String>,
}

/// Run node with logging
/// based on reth::cli::Cli::run
fn run<L>(
    mut command: NodeCommand<AlpenChainSpecParser, AdditionalConfig>,
    launcher: L,
) -> eyre::Result<()>
where
    L: std::ops::AsyncFnOnce(
        WithLaunchContext<NodeBuilder<Arc<reth_db::DatabaseEnv>, ChainSpec>>,
        AdditionalConfig,
    ) -> eyre::Result<()>,
{
    command.ext.logs.log_file_directory = command
        .ext
        .logs
        .log_file_directory
        .join(command.chain.chain.to_string());

    let _guard = command.ext.logs.init_tracing()?;
    info!(target: "reth::cli", cmd = %command.ext.logs.log_file_directory, "Initialized tracing, debug log directory");

    let runner = CliRunner::try_default_runtime()?;
    runner.run_command_until_exit(|ctx| {
        command.execute(
            ctx,
            FnLauncher::new::<AlpenChainSpecParser, AdditionalConfig>(launcher),
        )
    })?;

    Ok(())
}

async fn blockhash_sync_worker<N: NodeTypesWithDB + ProviderNodeTypes>(
    provider: BlockchainProvider<N>,
    mut rx: mpsc::Receiver<ConsensusEvent>,
    engine: impl ExecutionEngine<AlpenBuiltPayload>,
) {
    let mut state = ForkchoiceState {
        head_block_hash: provider.canonical_in_memory_state().chain_info().best_hash,
        safe_block_hash: provider.safe_block_hash().unwrap().unwrap_or(B256::ZERO),
        finalized_block_hash: provider
            .finalized_block_hash()
            .unwrap()
            .unwrap_or(B256::ZERO),
    };

    while let Some(ev) = rx.recv().await {
        let next_state = match ev {
            ConsensusEvent::Head(account_state_commitment) => ForkchoiceState {
                head_block_hash: account_state_commitment.inner(),
                safe_block_hash: B256::ZERO,
                finalized_block_hash: B256::ZERO,
            },
            ConsensusEvent::OlUpdated {
                confirmed,
                finalized,
            } => {
                let safe_block_hash: B256 = confirmed.inner();
                let finalized_block_hash: B256 = finalized.inner();

                // Check that safe and finalized block are in current canonical
                // chain
                match (
                    provider.block_number(safe_block_hash).unwrap(),
                    provider.block_number(finalized_block_hash).unwrap(),
                ) {
                    (Some(safe_block_num), Some(finalized_block_num))
                        if safe_block_num >= finalized_block_num =>
                    {
                        // happy path: both safe and finalized blocks are part
                        // of canonical chain, in expected order
                        ForkchoiceState {
                            head_block_hash: state.head_block_hash,
                            safe_block_hash,
                            finalized_block_hash,
                        }
                    }
                    (_, Some(_finalized_block_num)) => {
                        // Case 1: finalized > safe; should not be possible, but
                        // take both as finalized
                        // Case 2: safe block not found, finalized is still
                        // present in canonical chain
                        //
                        // Use finalized blockhash as safe blockhash
                        ForkchoiceState {
                            head_block_hash: state.head_block_hash,
                            safe_block_hash: finalized_block_hash,
                            finalized_block_hash,
                        }
                    }
                    (_, None) => {
                        // Finalized block not present in current canonical
                        // chain.
                        // TODO: reorg;
                        // FIXME:

                        warn!("OL reorg detected");

                        state
                    }
                }
            }
        };

        if let Err(err) = engine.update_consenesus_state(next_state.clone()).await {
            warn!("failed to update state; err: {}", err);

            continue;
        }

        state = next_state;
    }
}

async fn block_producer_worker(
    sequencer_config: SequencerConfig,
    chainstate_provider: impl ChainStateProvider,
    time: impl ClockProvider,
    engine: impl PayloadBuilderEngine<AlpenBuiltPayload>,
    tx: mpsc::Sender<ConsensusEvent>,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(
        sequencer_config.target_blocktime_ms as u64,
    ));
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        interval.tick().await;
        let build_ts = time.now_millis();

        let head_block_hash = chainstate_provider.head_block_hash().hash;

        // TODO: check batch production criteria, add deposits to payload attributes

        let build_attrs = EnginePayloadAttributes {
            parent: head_block_hash,
            timestamp: build_ts,
            deposits: vec![],
        };

        match build_next_block(&engine, build_attrs).await {
            Ok(new_blockhash) => {
                let _ = tx
                    .send(ConsensusEvent::Head(AccountStateCommitment::from(
                        new_blockhash,
                    )))
                    .await;
                // TODO: send new block hash to network peers
            }
            Err(err) => {
                warn!("err: {}", err);
            }
        };
    }
}

async fn build_next_block(
    engine: &impl PayloadBuilderEngine<AlpenBuiltPayload>,
    build_attrs: EnginePayloadAttributes,
) -> eyre::Result<B256> {
    let payload = engine.build_payload(build_attrs).await?;
    let new_blockhash = payload.block().hash();

    engine.submit_payload(payload).await?;

    Ok(new_blockhash)
}
