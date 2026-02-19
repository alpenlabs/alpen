//! Command line arguments for Strata's `datatool` binary.

use std::{num::NonZero, path::PathBuf};

use argh::FromArgs;
use bitcoin::Network;
use rand_core::OsRng;

use crate::util::resolve_network;

/// Args.
#[derive(FromArgs)]
pub(crate) struct Args {
    #[argh(option, description = "network name [signet, regtest]", short = 'b')]
    pub(crate) bitcoin_network: Option<String>,

    #[argh(
        option,
        description = "bitcoin RPC URL (required for Bitcoin operations when btc-client feature is enabled)"
    )]
    pub(crate) bitcoin_rpc_url: Option<String>,

    #[argh(
        option,
        description = "bitcoin RPC username (required for Bitcoin operations when btc-client feature is enabled)"
    )]
    pub(crate) bitcoin_rpc_user: Option<String>,

    #[argh(
        option,
        description = "bitcoin RPC password (required for Bitcoin operations when btc-client feature is enabled)"
    )]
    pub(crate) bitcoin_rpc_password: Option<String>,

    #[argh(
        option,
        description = "data directory (unused) (default cwd)",
        short = 'd'
    )]
    pub(crate) datadir: Option<PathBuf>,

    #[argh(subcommand)]
    pub(crate) subc: Subcommand,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand)]
pub(crate) enum Subcommand {
    Xpriv(SubcXpriv),
    SeqPubkey(SubcSeqPubkey),
    SeqPrivkey(SubcSeqPrivkey),
    Params(SubcParams),
    AsmParams(SubcAsmParams),
    #[cfg(feature = "btc-client")]
    GenL1View(SubcGenL1View),
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(
    subcommand,
    name = "genxpriv",
    description = "generates a master xpriv and writes it to a file"
)]
pub(crate) struct SubcXpriv {
    #[argh(positional, description = "output path")]
    pub(crate) path: PathBuf,

    #[argh(switch, description = "force overwrite", short = 'f')]
    pub(crate) force: bool,
}

/// Generate the sequencer pubkey to pass around.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(
    subcommand,
    name = "genseqpubkey",
    description = "generates a sequencer pubkey from a master xpriv"
)]
pub(crate) struct SubcSeqPubkey {
    #[argh(option, description = "reads key from specified file", short = 'f')]
    pub(crate) key_file: Option<PathBuf>,

    #[argh(
        switch,
        description = "reads key from envvar STRATA_SEQ_KEY",
        short = 'E'
    )]
    pub(crate) key_from_env: bool,
}

/// Generate the sequencer pubkey to pass around.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(
    subcommand,
    name = "genseqprivkey",
    description = "generates a sequencer privkey from a master xpriv"
)]
pub(crate) struct SubcSeqPrivkey {
    #[argh(option, description = "reads key from specified file", short = 'f')]
    pub(crate) key_file: Option<PathBuf>,

    #[argh(
        switch,
        description = "reads key from envvar STRATA_SEQ_KEY",
        short = 'E'
    )]
    pub(crate) key_from_env: bool,
}

/// Generate a network's param file from inputs.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(
    subcommand,
    name = "genparams",
    description = "generates network params from inputs"
)]
pub(crate) struct SubcParams {
    #[argh(
        option,
        description = "output file path .json (default stdout)",
        short = 'o'
    )]
    pub(crate) output: Option<PathBuf>,

    #[argh(
        option,
        description = "network name, used for magics (default random)",
        short = 'n'
    )]
    pub(crate) name: Option<String>,

    #[argh(
        option,
        description = "sequencer pubkey (default unchecked)",
        short = 's'
    )]
    pub(crate) seqkey: Option<String>,

    #[argh(
        option,
        description = "add a bridge operator key (master xpriv, must be at least one, appended after file keys)",
        short = 'b'
    )]
    pub(crate) opkey: Vec<String>,

    #[argh(
        option,
        description = "read bridge operator keys (master xpriv) by line from file",
        short = 'B'
    )]
    pub(crate) opkeys: Option<PathBuf>,

    #[argh(option, description = "deposit amount in sats (default \"10 BTC\")")]
    pub(crate) deposit_sats: Option<String>,

    #[argh(
        option,
        description = "genesis L1 block height (default 100)",
        short = 'g'
    )]
    pub(crate) genesis_l1_height: Option<u64>,

    #[argh(option, description = "block time in seconds (default 5)", short = 't')]
    pub(crate) block_time: Option<u64>,

    #[argh(option, description = "epoch duration in slots (default 64)")]
    pub(crate) epoch_slots: Option<u32>,

    #[argh(
        option,
        description = "permit blank proofs after timeout in millis (default strict)"
    )]
    pub(crate) proof_timeout: Option<u32>,

    #[argh(option, description = "directory to export the generated ELF")]
    pub(crate) elf_dir: Option<PathBuf>,

    #[argh(option, description = "path to evm chain config json")]
    pub(crate) chain_config: Option<PathBuf>,

    #[argh(
        option,
        description = "path to JSON-serialized genesis L1 view (required when btc-client feature is disabled)"
    )]
    pub(crate) genesis_l1_view_file: Option<String>,
}

/// Generate an ASM params file from inputs.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(
    subcommand,
    name = "gen-asm-params",
    description = "generates ASM params from inputs"
)]
pub(crate) struct SubcAsmParams {
    #[argh(
        option,
        description = "output file path .json (default stdout)",
        short = 'o'
    )]
    pub(crate) output: Option<PathBuf>,

    #[argh(
        option,
        description = "network name / magic bytes (default ALPN)",
        short = 'n'
    )]
    pub(crate) name: Option<String>,

    #[argh(
        option,
        description = "add a bridge operator key (master xpriv)",
        short = 'b'
    )]
    pub(crate) opkey: Vec<String>,

    #[argh(
        option,
        description = "read bridge operator keys (master xpriv) by line from file",
        short = 'B'
    )]
    pub(crate) opkeys: Option<PathBuf>,

    #[argh(option, description = "deposit amount in sats (default \"10 BTC\")")]
    pub(crate) deposit_sats: Option<String>,

    #[argh(
        option,
        description = "genesis L1 block height (default 100)",
        short = 'g'
    )]
    pub(crate) genesis_l1_height: Option<u64>,

    #[argh(
        option,
        description = "path to JSON-serialized genesis L1 view (required when btc-client feature is disabled)"
    )]
    pub(crate) genesis_l1_view_file: Option<String>,

    #[argh(option, description = "assignment duration in blocks (default 64)")]
    pub(crate) assignment_duration: Option<u16>,

    #[argh(option, description = "recovery delay in blocks (default 1008)")]
    pub(crate) recovery_delay: Option<u16>,

    #[argh(option, description = "operator fee in sats (default 50000000)")]
    pub(crate) operator_fee: Option<u64>,

    #[argh(
        option,
        description = "confirmation depth for admin subprotocol (default 144)"
    )]
    pub(crate) confirmation_depth: Option<u16>,

    #[argh(
        option,
        description = "confirmation depth for admin subprotocol (default 100)"
    )]
    pub(crate) max_seqno_gap: Option<NonZero<u8>>,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(
    subcommand,
    name = "genl1view",
    description = "generates the genesis L1 view at the given height"
)]
pub(crate) struct SubcGenL1View {
    #[argh(option, description = "genesis L1 block height", short = 'g')]
    pub(crate) genesis_l1_height: u64,

    #[argh(
        option,
        description = "output file path .json (default stdout)",
        short = 'o'
    )]
    pub(crate) output: Option<PathBuf>,
}

/// Bitcoin RPC connection configuration.
pub(crate) struct BitcoindConfig {
    pub(crate) rpc_url: String,
    pub(crate) rpc_user: String,
    pub(crate) rpc_password: String,
}

pub(crate) struct CmdContext {
    /// Resolved datadir for the network.
    #[expect(
        unused,
        reason = "Field is used in command context but may not be directly accessed"
    )]
    pub(crate) datadir: PathBuf,

    /// The Bitcoin network we're building on top of.
    pub(crate) bitcoin_network: Network,

    /// Shared RNG, must be a cryptographically secure, high-entropy RNG.
    pub(crate) rng: OsRng,

    /// Bitcoin RPC configuration (None if credentials not provided).
    pub(crate) bitcoind_config: Option<BitcoindConfig>,
}

/// Resolves the command context and subcommand from the parsed command line arguments.
pub(crate) fn resolve_context_and_subcommand(
    args: Args,
) -> anyhow::Result<(CmdContext, Subcommand)> {
    let network = resolve_network(args.bitcoin_network.as_deref())?;

    let bitcoind_config = create_bitcoind_config(&args);

    let ctx = CmdContext {
        datadir: args.datadir.unwrap_or_else(|| PathBuf::from(".")),
        bitcoin_network: network,
        rng: OsRng,
        bitcoind_config,
    };

    Ok((ctx, args.subc))
}

/// Creates a Bitcoin RPC configuration if all required credentials are provided.
///
/// Returns `Some(BitcoindConfig)` if URL, username, and password are all provided,
/// otherwise returns `None`.
fn create_bitcoind_config(args: &Args) -> Option<BitcoindConfig> {
    match (
        &args.bitcoin_rpc_url,
        &args.bitcoin_rpc_user,
        &args.bitcoin_rpc_password,
    ) {
        (Some(url), Some(user), Some(password)) => Some(BitcoindConfig {
            rpc_url: url.clone(),
            rpc_user: user.clone(),
            rpc_password: password.clone(),
        }),
        _ => None,
    }
}
