//! Reth node for the Alpen codebase.

mod genesis;
mod gossip;
mod ol_client;

use std::{env, process, sync::Arc};

use alpen_chainspec::{chain_value_parser, AlpenChainSpecParser};
use alpen_ee_common::traits::ol_client::chain_status_checked;
use alpen_ee_config::{AlpenEeConfig, AlpenEeParams};
use alpen_ee_database::init_db_storage;
use alpen_ee_engine::{create_engine_control_task, AlpenRethExecEngine};
use alpen_ee_ol_tracker::{init_ol_tracker_state, OlTrackerBuilder};
use alpen_reth_node::{
    args::AlpenNodeArgs, AlpenEthereumNode, AlpenGossipProtocolHandler, AlpenGossipState,
};
use clap::Parser;
use ol_client::DummyOlClient;
use reth_chainspec::ChainSpec;
use reth_cli_commands::{launcher::FnLauncher, node::NodeCommand};
use reth_cli_runner::CliRunner;
use reth_cli_util::sigsegv_handler;
use reth_network::{protocol::IntoRlpxSubProtocol, NetworkProtocols};
use reth_node_builder::{NodeBuilder, WithLaunchContext};
use reth_node_core::args::LogArgs;
use reth_provider::CanonStateSubscriptions;
use strata_acct_types::AccountId;
use strata_identifiers::{CredRule, OLBlockId};
use strata_primitives::buf::Buf32;
use tokio::sync::{broadcast, mpsc};
use tracing::info;

use crate::{
    genesis::ee_genesis_block_info,
    gossip::{create_gossip_task, GossipConfig},
};

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
    // disable peer discovery
    command.network.discovery.disable_discovery = true;
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
            let datadir = builder.config().datadir().data_dir().to_path_buf();

            let genesis_info = ee_genesis_block_info(&ext.custom_chain);

            let params = AlpenEeParams::new(
                AccountId::new([0; 32]),
                genesis_info.blockhash(),
                genesis_info.stateroot(),
                0,
                OLBlockId::null(), // TODO
            );

            let config = Arc::new(AlpenEeConfig::new(
                params,
                CredRule::Unchecked,
                ext.ol_client_http,
                ext.sequencer_http,
                ext.db_retry_count,
            ));

            let storage: Arc<_> = init_db_storage(&datadir, config.db_retry_count())
                .expect("failed to load alpen database")
                .into();

            // TODO: real ol client
            let ol_client = Arc::new(DummyOlClient::default());

            // TODO: startup consistency check
            let ol_chain_status = builder
                .task_executor()
                .handle()
                .block_on(chain_status_checked(ol_client.as_ref()))
                .expect("cannot fetch OL chain status");

            let ol_tracker_state = builder
                .task_executor()
                .handle()
                .block_on(init_ol_tracker_state(
                    config.clone(),
                    ol_chain_status,
                    storage.clone(),
                ))
                .expect("ol tracker state initialization should not fail");

            // Create gossip channel before building the node so we can register it early
            let (gossip_tx, gossip_rx) = mpsc::unbounded_channel();

            // Create preconf channel for p2p head block gossip -> engine control integration
            // This channel sends block hashes received from peers to the engine control task
            let (preconf_tx, preconf_rx) = broadcast::channel(16);

            let node_builder = builder
                .node(AlpenEthereumNode::new(AlpenNodeArgs::default()))
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
                })
                .on_node_started(move |node| {
                    let (ol_tracker, ol_tracker_task) = OlTrackerBuilder::new(
                        ol_tracker_state,
                        config.params().clone(),
                        storage,
                        ol_client,
                    )
                    .build();

                    let engine_control_task = create_engine_control_task(
                        preconf_rx,
                        ol_tracker.consensus_watcher(),
                        node.provider.clone(),
                        AlpenRethExecEngine::new(node.beacon_engine_handle.clone()),
                    );

                    // Subscribe to canonical state notifications for broadcasting new blocks
                    let state_events = node.provider.subscribe_to_canonical_state();

                    // Parse sequencer private key from environment variable (only in sequencer mode)
                    #[cfg(feature = "sequencer")]
                    let gossip_config = {
                        let privkey_str = env::var("SEQUENCER_PRIVATE_KEY")
                            .map_err(|_| eyre::eyre!("SEQUENCER_PRIVATE_KEY environment variable is required when sequencer feature is enabled"))?;
                        let sequencer_privkey = privkey_str
                            .parse::<Buf32>()
                            .map_err(|e| eyre::eyre!("Failed to parse SEQUENCER_PRIVATE_KEY as hex: {e}"))?;
                        GossipConfig {
                            sequencer_pubkey: ext.sequencer_pubkey,
                            sequencer_privkey,
                        }
                    };

                    #[cfg(not(feature = "sequencer"))]
                    let gossip_config = GossipConfig {
                        sequencer_pubkey: ext.sequencer_pubkey,
                    };

                    let gossip_task = create_gossip_task(gossip_rx, state_events, preconf_tx, gossip_config);

                    node.task_executor
                        .spawn_critical("ol_tracker_task", ol_tracker_task);
                    node.task_executor
                        .spawn_critical("engine_control", engine_control_task);
                    node.task_executor
                        .spawn_critical("gossip_task", gossip_task);

                    // sequencer specific tasks
                    // TODO: block assembly
                    // TODO: batch assembly
                    // TODO: proof generation
                    // TODO: post update to OL

                    Ok(())
                });

            let handle = node_builder.launch().await?;
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

    /// Rpc of sequencer's reth node to forward transactions to.
    #[arg(long, required = false)]
    pub sequencer_http: Option<String>,

    /// Rpc of OL node.
    #[arg(long, required = true)]
    pub ol_client_http: String,

    #[arg(long, required = false)]
    pub db_retry_count: Option<u16>,

    /// Sequencer's public key (hex-encoded, 32 bytes) for signature validation.
    #[arg(long, required = true, value_parser = parse_buf32)]
    pub sequencer_pubkey: Buf32,
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

/// Parse a hex-encoded string into a [`Buf32`].
fn parse_buf32(s: &str) -> eyre::Result<Buf32> {
    s.parse::<Buf32>()
        .map_err(|e| eyre::eyre!("Failed to parse hex string as Buf32: {e}"))
}
