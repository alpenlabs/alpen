pub mod get_syncinfo;
pub mod reset_chainstate;

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Alpen DB tool – offline database & chain‑maintenance utility.
#[derive(Parser, Debug)]
#[command(
    name = "alpen-dbtool",
    version,
    about = "Inspect, repair and roll back an Alpen node’s database while the node is offline.",
    propagate_version = true,
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Node data directory (same as `--datadir` used by the node).
    #[arg(short = 'd', long = "datadir", value_name = "PATH")]
    pub datadir: PathBuf,

    /// Back‑end DB implementation (rocksdb | sled).
    #[arg(short = 't', long = "type", default_value = "rocksdb")]
    pub db_type: String,

    /// Sub‑command selector.
    #[command(subcommand)]
    pub cmd: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Show node’s sync progress on L1 and L2.
    GetSyncinfo(GetSyncinfo),

    /// Roll back chainstate to a particular L2 block (epoch‑terminal by default).
    ResetChainstate(ResetChainstate),
}

#[derive(Args, Debug)]
pub struct GetSyncinfo {
    /// Emit structured JSON instead of human‑readable output.
    #[arg(short = 'p', long = "porcelain")]
    porcelain: bool,
}

#[derive(Args, Debug)]
pub struct ResetChainstate {
    /// Target L2 block hash or number to roll back to.
    #[arg(value_name = "L2_BLOCK_ID")]
    pub block_id: String,

    /// Allow resetting to a non‑epoch‑terminal block (dangerous).
    #[arg(long = "allow-non-terminal")]
    pub allow_nterm: bool,
}
