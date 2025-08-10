//! Command line arguments for Strata's `datatool` binary.

use std::path::PathBuf;

use argh::FromArgs;
use bitcoin::Network;
use rand_core::OsRng;

/// Args.
#[derive(FromArgs)]
pub(crate) struct Args {
    #[argh(option, description = "network name [signet, regtest]", short = 'b')]
    pub(crate) bitcoin_network: Option<String>,

    #[argh(
        option,
        description = "data directory (unused) (default cwd)",
        short = 'd'
    )]
    pub(crate) datadir: Option<PathBuf>,

    #[argh(subcommand)]
    pub(crate) subc: Subcommand,
}

#[allow(clippy::large_enum_variant)]
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand)]
pub(crate) enum Subcommand {
    Xpriv(SubcXpriv),
    SeqPubkey(SubcSeqPubkey),
    SeqPrivkey(SubcSeqPrivkey),
    OpXpub(SubcOpXpub),
    Params(SubcParams),
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

/// Generate operator xpub to pass around.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(
    subcommand,
    name = "genopxpub",
    description = "generates an operator xpub from a master xpriv"
)]
pub(crate) struct SubcOpXpub {
    #[argh(option, description = "reads key from specified file", short = 'f')]
    pub(crate) key_file: Option<PathBuf>,

    #[argh(
        switch,
        description = "reads key from envvar STRATA_OP_KEY",
        short = 'E'
    )]
    pub(crate) key_from_env: bool,

    #[argh(switch, description = "print the p2p key", short = 'p')]
    pub(crate) p2p: bool,

    #[argh(switch, description = "print the wallet key", short = 'w')]
    pub(crate) wallet: bool,
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
        description = "DA tag, used in envelopes (default 'strata-da')"
    )]
    pub(crate) da_tag: Option<String>,

    #[argh(
        option,
        description = "checkpoint tag, used in envelopes (default 'strata-ckpt')"
    )]
    pub(crate) checkpoint_tag: Option<String>,

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

    #[argh(option, description = "horizon height (default 90)", short = 'h')]
    pub(crate) horizon_height: Option<u64>,

    #[argh(option, description = "genesis L1 block height (required)", short = 'H')]
    pub(crate) genesis_l1_height: u64,

    #[argh(option, description = "genesis L1 block hash (required)", short = 'G')]
    pub(crate) genesis_l1_hash: String,

    #[argh(
        option,
        description = "block time in seconds (default 15)",
        short = 't'
    )]
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
}

pub(crate) struct CmdContext {
    /// Resolved datadir for the network.
    #[allow(unused)]
    pub(crate) datadir: PathBuf,

    /// The Bitcoin network we're building on top of.
    pub(crate) bitcoin_network: Network,

    /// Shared RNG, must be a cryptographically secure, high-entropy RNG.
    pub(crate) rng: OsRng,
}
