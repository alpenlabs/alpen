pub mod alpen;
pub mod chainstate;
pub mod checkpoint;
pub mod epoch;
pub mod syncinfo;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::cmd::{
    alpen::GetAlpenBlockArgs, chainstate::ResetChainstateArgs, checkpoint::GetCheckpointDataArgs,
    epoch::GetEpochSummaryArgs, syncinfo::GetSyncinfoArgs,
};

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
    /// Show Alpen block header.
    GetAlpenBlock(GetAlpenBlockArgs),

    /// Show checkpoint data.
    GetCheckpointData(GetCheckpointDataArgs),

    /// Show epoch summary.
    GetEpochSummary(GetEpochSummaryArgs),

    /// Show node’s sync progress on Alpen and Signet.
    GetSyncinfo(GetSyncinfoArgs),

    /// Roll back chainstate to a particular Alpen block (epoch‑terminal by default).
    ResetChainstate(ResetChainstateArgs),
}
