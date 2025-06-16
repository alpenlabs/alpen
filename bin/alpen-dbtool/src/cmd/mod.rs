pub mod chainstate;
pub mod checkpoint;
pub mod epoch;
pub mod l1_manifest;
pub mod l2_block;
pub mod syncinfo;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::cmd::{
    chainstate::ResetChainstateArgs, checkpoint::GetCheckpointDataArgs, epoch::GetEpochSummaryArgs,
    l1_manifest::GetL1ManifestArgs, l2_block::GetL2BlockArgs, syncinfo::GetSyncinfoArgs,
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
    /// Show L1 block manifest.
    GetL1Manifest(GetL1ManifestArgs),

    /// Show L2 block.
    GetL2Block(GetL2BlockArgs),

    /// Show checkpoint data.
    GetCheckpointData(GetCheckpointDataArgs),

    /// Show epoch summary.
    GetEpochSummary(GetEpochSummaryArgs),

    /// Show node’s sync progress on Alpen and Signet.
    GetSyncinfo(GetSyncinfoArgs),

    /// Roll back chainstate to a particular Alpen block (epoch‑terminal by default).
    ResetChainstate(ResetChainstateArgs),
}
