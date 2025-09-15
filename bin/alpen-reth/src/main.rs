//! Reth node for the Alpen codebase.

mod init_db;

use std::{collections::HashMap, process, sync::Arc};

use alpen_chainspec::{chain_value_parser, AlpenChainSpecParser};
use alpen_reth_exex::{ProverWitnessGenerator, StateDiffGenerator};
use alpen_reth_node::{
    args::AlpenNodeArgs,
    head_gossip::{
        connection::HeadGossipCommand,
        handler::{HeadGossipEvent, HeadGossipProtocolHandler, HeadGossipState},
    },
    AlpenEthereumNode,
};
use alpen_reth_rpc::{AlpenRPC, StrataRpcApiServer};
use clap::Parser;
use futures_util::StreamExt;
use init_db::init_witness_db;
use reth_chainspec::ChainSpec;
use reth_cli_commands::{launcher::FnLauncher, node::NodeCommand};
use reth_cli_runner::CliRunner;
use reth_network::{protocol::IntoRlpxSubProtocol, NetworkProtocols};
use reth_network_api::PeerId;
use reth_node_builder::{NodeBuilder, WithLaunchContext};
use reth_node_core::args::LogArgs;
use reth_primitives::Header;
use reth_transaction_pool::{FullTransactionEvent, TransactionPool};
use tokio::sync::mpsc;
use tracing::info;

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

    if let Err(err) = run(
        command,
        |builder: WithLaunchContext<NodeBuilder<Arc<reth_db::DatabaseEnv>, ChainSpec>>,
         ext: AdditionalConfig| async move {
            let datadir = builder.config().datadir().data_dir().to_path_buf();

            let node_args = AlpenNodeArgs {
                sequencer_http: ext.sequencer_http.clone(),
            };

            let mut node_builder = builder.node(AlpenEthereumNode::new(node_args));

            let mut extend_rpc = None;

            if ext.enable_witness_gen || ext.enable_state_diff_gen {
                let db = init_witness_db(&datadir).expect("initialize witness database");
                // Add RPC for querying block witness and state diffs.
                extend_rpc.replace(AlpenRPC::new(db.clone()));
                // Install Prover Input ExEx and persist to DB
                if ext.enable_witness_gen {
                    let witness_db = db.clone();
                    node_builder = node_builder.install_exex("prover_input", |ctx| async {
                        Ok(ProverWitnessGenerator::new(ctx, witness_db).start())
                    });
                }

                // Install State Diff ExEx and persist to DB
                if ext.enable_state_diff_gen {
                    let state_diff_db = db.clone();
                    node_builder = node_builder.install_exex("state_diffs", |ctx| async {
                        Ok(StateDiffGenerator::new(ctx, state_diff_db).start())
                    });
                }
            }

            // Note: can only add single hook
            node_builder = node_builder.extend_rpc_modules(|ctx| {
                if let Some(rpc) = extend_rpc {
                    ctx.modules.merge_configured(rpc.into_rpc())?;
                }

                Ok(())
            });

            let handle = node_builder.launch().await?;

            // Create and add the custom RLPx subprotocol
            // See: crates/reth/node/src/head_gossip/
            let (gossip_tx, mut gossip_rx) = mpsc::unbounded_channel();
            let handler = HeadGossipProtocolHandler {
                state: HeadGossipState { events: gossip_tx },
            };
            handle
                .node
                .network
                .add_rlpx_sub_protocol(handler.into_rlpx_sub_protocol());

            // This task will handle gossip events
            let mut tx_events = handle.node.pool.all_transactions_event_listener();
            tokio::spawn(async move {
                let mut connections: HashMap<PeerId, mpsc::UnboundedSender<HeadGossipCommand>> =
                    HashMap::new();
                loop {
                    tokio::select! {
                        Some(event) = gossip_rx.recv() => {
                            match event {
                                HeadGossipEvent::Established {
                                    peer_id,
                                    to_connection,
                                    ..
                                } => {
                                    connections.insert(peer_id, to_connection);
                                }
                                HeadGossipEvent::Closed { peer_id } => {
                                    connections.remove(&peer_id);
                                }
                                HeadGossipEvent::HeadHash { peer_id, header } => {
                                    // TODO(@storopoli): do something when we receive a single `Header`.
                                    info!(target: "head-gossip", "Received head hash from peer {}: {:?}", peer_id, header.hash_slow());
                                }
                                HeadGossipEvent::HeadHashes { peer_id, headers } => {
                                    // TODO(@storopoli): do something when we receive multiple `Header`s.
                                    info!(target: "head-gossip", "Received {} head hashes from peer {}", headers.len(), peer_id);
                                }
                            }
                        },
                        Some(event) = tx_events.next() => {
                            // TODO(@storopoli): How to get a `Header`?
                            if let FullTransactionEvent::Mined { block_hash, .. } = event {
                                info!(target: "head-gossip", "New block mined: {:?}, broadcasting to {} peers", block_hash, connections.len());
                                let header = Header::default();
                                for sender in connections.values() {
                                    let _ = sender.send(HeadGossipCommand::SendHeadHash(header.clone()));
                                }
                            }
                        }
                    }
                }
            });

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

    #[arg(long, default_value_t = false)]
    pub enable_witness_gen: bool,

    #[arg(long, default_value_t = false)]
    pub enable_state_diff_gen: bool,

    /// Rpc of sequencer's reth node to forward transactions to.
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
