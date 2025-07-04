use std::{fmt, path::PathBuf, str::FromStr};

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
/// Inspect, repair and roll back an Alpen node’s database while the node is offline."
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

/// Output format
#[derive(PartialEq, Eq, Debug, Clone)]
pub(crate) enum OutputFormat {
    /// Structured JSON
    Json,
    /// Similar to porcelain in git
    Porcelain,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct UnsupportedOutputFormat;

impl fmt::Display for UnsupportedOutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "must be 'json' or 'porcelain'")
    }
}

impl FromStr for OutputFormat {
    type Err = UnsupportedOutputFormat;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "json" => Ok(Self::Json),
            "porcelain" => Ok(Self::Porcelain),
            _ => Err(UnsupportedOutputFormat),
        }
    }
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            OutputFormat::Json => "json",
            OutputFormat::Porcelain => "porcelain",
        })
    }
}
