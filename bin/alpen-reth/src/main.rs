mod db;

use std::{fs, future::Future, path::PathBuf, sync::Arc};

use alloy_genesis::Genesis;
use alpen_reth_db::rocksdb::WitnessDB;
use alpen_reth_exex::ProverWitnessGenerator;
use alpen_reth_node::{args::StrataNodeArgs, StrataEthereumNode};
use alpen_reth_rpc::{StrataRPC, StrataRpcApiServer};
use clap::Parser;
use reth::{
    args::LogArgs,
    builder::{NodeBuilder, WithLaunchContext},
    CliRunner,
};
use reth_chainspec::ChainSpec;
use reth_cli::chainspec::ChainSpecParser;
use reth_cli_commands::node::NodeCommand;
use tracing::info;

const DEFAULT_CHAIN_SPEC: &str = include_str!("../res/testnet-chain.json");
const DEVNET_CHAIN_SPEC: &str = include_str!("../res/devnet-chain.json");
const DEV_CHAIN_SPEC: &str = include_str!("../res/alpen-dev-chain.json");

fn main() {
    reth_cli_util::sigsegv_handler::install();

    // Enable backtraces unless a RUST_BACKTRACE value has already been explicitly provided.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        std::env::set_var("RUST_BACKTRACE", "1");
    }

    let mut command = NodeCommand::<StrataChainSpecParser, AdditionalConfig>::parse();

    // use provided alpen chain spec
    command.chain = command.ext.custom_chain.clone();
    // disable peer discovery
    command.network.discovery.disable_discovery = true;
    // allow chain fork blocks to be created
    command.always_process_payload_attributes_on_canonical_head = true;

    if let Err(err) = run(command, |builder, ext| async move {
        let datadir = builder.config().datadir().data_dir().to_path_buf();

        let node_args = StrataNodeArgs {
            sequencer_http: ext.sequencer_http.clone(),
        };

        let mut node_builder = builder.node(StrataEthereumNode::new(node_args));

        let mut extend_rpc = None;

        // Install Prover Input ExEx, persist to DB, and add RPC for querying block witness.
        if ext.enable_witness_gen {
            let rbdb = db::open_rocksdb_database(datadir.clone()).expect("open rocksdb");
            let db = Arc::new(WitnessDB::new(rbdb));
            let rpc_db = db.clone();

            extend_rpc.replace(StrataRPC::new(rpc_db));

            node_builder = node_builder.install_exex("prover_input", |ctx| async {
                Ok(ProverWitnessGenerator::new(ctx, db).start())
            });
        }

        // Note: can only add single hook
        node_builder = node_builder.extend_rpc_modules(|ctx| {
            if let Some(rpc) = extend_rpc {
                ctx.modules.merge_configured(rpc.into_rpc())?;
            }

            Ok(())
        });

        let handle = node_builder.launch().await?;
        handle.node_exit_future.await
    }) {
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

    /// Rpc of sequener's reth node to forward transactions to.
    #[arg(long, required = false)]
    pub sequencer_http: Option<String>,
}

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct StrataChainSpecParser;

impl ChainSpecParser for StrataChainSpecParser {
    type ChainSpec = ChainSpec;

    const SUPPORTED_CHAINS: &'static [&'static str] = &["dev", "devnet", "testnet"];

    fn parse(s: &str) -> eyre::Result<Arc<Self::ChainSpec>> {
        chain_value_parser(s)
    }
}

pub fn chain_value_parser(s: &str) -> eyre::Result<Arc<ChainSpec>, eyre::Error> {
    Ok(match s {
        "testnet" => parse_chain_spec(DEFAULT_CHAIN_SPEC)?,
        "devnet" => parse_chain_spec(DEVNET_CHAIN_SPEC)?,
        "dev" => parse_chain_spec(DEV_CHAIN_SPEC)?,
        _ => {
            // try to read json from path first
            let raw = match fs::read_to_string(PathBuf::from(shellexpand::full(s)?.into_owned())) {
                Ok(raw) => raw,
                Err(io_err) => {
                    // valid json may start with "\n", but must contain "{"
                    if s.contains('{') {
                        s.to_string()
                    } else {
                        return Err(io_err.into()); // assume invalid path
                    }
                }
            };

            // both serialized Genesis and ChainSpec structs supported
            let genesis: Genesis = serde_json::from_str(&raw)?;

            Arc::new(genesis.into())
        }
    })
}

fn parse_chain_spec(chain_json: &str) -> eyre::Result<Arc<ChainSpec>> {
    // both serialized Genesis and ChainSpec structs supported
    let genesis: Genesis = serde_json::from_str(chain_json)?;

    Ok(Arc::new(genesis.into()))
}

/// Run node with logging
/// based on reth::cli::Cli::run
fn run<L, Fut>(
    mut command: NodeCommand<StrataChainSpecParser, AdditionalConfig>,
    launcher: L,
) -> eyre::Result<()>
where
    L: FnOnce(
        WithLaunchContext<NodeBuilder<Arc<reth_db::DatabaseEnv>, ChainSpec>>,
        AdditionalConfig,
    ) -> Fut,
    Fut: Future<Output = eyre::Result<()>>,
{
    command.ext.logs.log_file_directory = command
        .ext
        .logs
        .log_file_directory
        .join(command.chain.chain.to_string());

    let _guard = command.ext.logs.init_tracing()?;
    info!(target: "reth::cli", cmd = %command.ext.logs.log_file_directory, "Initialized tracing, debug log directory");

    let runner = CliRunner::default();
    runner.run_command_until_exit(|ctx| command.execute(ctx, launcher))?;

    Ok(())
}
