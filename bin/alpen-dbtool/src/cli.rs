use std::path::PathBuf;

use clap::{crate_version, Parser, Subcommand};

use crate::cmd::{
    chainstate::{GetChainstateArgs, ResetChainstateArgs},
    checkpoint::{GetCheckpointDataArgs, GetCheckpointsSummaryArgs, GetEpochSummaryArgs},
    client_state::GetClientStateArgs,
    l1::{GetL1ManifestArgs, GetL1SummaryArgs},
    l2::GetL2BlockArgs,
    sync_event::{GetSyncEventArgs, GetSyncEventsSummaryArgs},
    syncinfo::GetSyncinfoArgs,
};

/// Alpen DB tool – offline database & chain‑maintenance utility.
#[derive(Parser, Debug)]
#[command(
    name = "alpen-dbtool",
    version = crate_version!(),
    about = "Inspect, repair and roll back an Alpen node’s database while the node is offline.",
    propagate_version = true,
    disable_help_subcommand = true
)]
pub(crate) struct Cli {
    /// Node data directory (same as `--datadir` used by the node).
    #[arg(
        short = 'd',
        long = "datadir",
        value_name = "PATH",
        default_value = "data"
    )]
    pub(crate) datadir: PathBuf,

    /// Back‑end DB implementation (rocksdb | sled).
    #[arg(short = 't', long = "type", default_value = "rocksdb")]
    pub(crate) db_type: String,

    /// Sub‑command selector.
    #[command(subcommand)]
    pub(crate) cmd: Command,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    /// Show L1 block manifest.
    GetL1Manifest(GetL1ManifestArgs),

    /// Show L1 data summary.
    GetL1Summary(GetL1SummaryArgs),

    /// Show L2 block.
    GetL2Block(GetL2BlockArgs),

    /// Show latest client state update.
    GetClientState(GetClientStateArgs),

    /// Show checkpoint data.
    GetCheckpointData(GetCheckpointDataArgs),

    /// Show summary of checkpoints.
    GetCheckpointsSummary(GetCheckpointsSummaryArgs),

    /// Show epoch summary.
    GetEpochSummary(GetEpochSummaryArgs),

    /// Show node’s sync progress on Alpen and Signet.
    GetSyncinfo(GetSyncinfoArgs),

    /// Show details about a sync event.
    GetSyncEvent(GetSyncEventArgs),

    /// Show summary of sync events.
    GetSyncEventsSummary(GetSyncEventsSummaryArgs),

    /// Get chainstate write batch.
    GetChainstate(GetChainstateArgs),

    /// Roll back chainstate to a particular Alpen block (epoch‑terminal by default).
    ResetChainstate(ResetChainstateArgs),
}
