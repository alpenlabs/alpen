use std::path::PathBuf;

use argh::FromArgs;

use crate::cmd::{
    chainstate::{GetChainstateArgs, ResetChainstateArgs},
    checkpoint::{GetCheckpointDataArgs, GetCheckpointsSummaryArgs, GetEpochSummaryArgs},
    client_state::GetClientStateUpdateArgs,
    l1::{GetL1ManifestArgs, GetL1SummaryArgs},
    l2::GetL2BlockArgs,
    sync_event::{GetSyncEventArgs, GetSyncEventsSummaryArgs},
    syncinfo::GetSyncinfoArgs,
};

/// Alpen DB tool – offline database & chain‑maintenance utility.
#[derive(FromArgs)]
#[argh(
    description = "Inspect, repair and roll back an Alpen node’s database while the node is offline."
)]
pub(crate) struct Cli {
    /// node data directory (same as `--datadir` used by the node).
    #[argh(option, short = 'd', default = "PathBuf::from(\"data\")")]
    pub(crate) datadir: PathBuf,

    /// back‑end DB implementation (rocksdb | sled).
    #[argh(option, short = 't', default = "String::from(\"rocksdb\")")]
    pub(crate) db_type: String,

    #[argh(subcommand)]
    pub(crate) cmd: Command,
}

/// Subcommand variants.
#[derive(FromArgs, Debug)]
#[argh(subcommand)]
pub(crate) enum Command {
    GetL1Manifest(GetL1ManifestArgs),
    GetL1Summary(GetL1SummaryArgs),
    GetL2Block(GetL2BlockArgs),
    GetClientStateUpdate(GetClientStateUpdateArgs),
    GetCheckpointData(GetCheckpointDataArgs),
    GetCheckpointsSummary(GetCheckpointsSummaryArgs),
    GetEpochSummary(GetEpochSummaryArgs),
    GetSyncinfo(GetSyncinfoArgs),
    GetSyncEvent(GetSyncEventArgs),
    GetSyncEventsSummary(GetSyncEventsSummaryArgs),
    GetChainstate(GetChainstateArgs),
    ResetChainstate(ResetChainstateArgs),
}
