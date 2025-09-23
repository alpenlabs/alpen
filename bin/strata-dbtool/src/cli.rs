use std::{path::PathBuf, str::FromStr};

use argh::FromArgs;

use crate::cmd::{
    broadcaster::{GetBroadcasterSummaryArgs, GetBroadcasterTxArgs},
    chainstate::{GetChainstateArgs, RevertChainstateArgs},
    checkpoint::{GetCheckpointArgs, GetCheckpointsSummaryArgs, GetEpochSummaryArgs},
    client_state::GetClientStateUpdateArgs,
    l1::{GetL1ManifestArgs, GetL1SummaryArgs},
    l2::{GetL2BlockArgs, GetL2SummaryArgs},
    syncinfo::GetSyncinfoArgs,
    writer::{GetWriterPayloadArgs, GetWriterSummaryArgs},
};

/// Strata DB tool – offline database & chain‑maintenance utility.
#[derive(FromArgs)]
/// Inspect, repair and roll back an Strata node's database while the node is offline.
pub(crate) struct Cli {
    /// node data directory (same as `--datadir` used by the node).
    #[argh(option, short = 'd', default = "PathBuf::from(\"data\")")]
    pub(crate) datadir: PathBuf,

    /// back‑end DB implementation (sled).
    #[argh(option, short = 't', default = "String::from(\"sled\")")]
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
    GetWriterSummary(GetWriterSummaryArgs),
    GetWriterPayload(GetWriterPayloadArgs),
    GetBroadcasterSummary(GetBroadcasterSummaryArgs),
    GetBroadcasterTx(GetBroadcasterTxArgs),
    GetL2Block(GetL2BlockArgs),
    GetL2Summary(GetL2SummaryArgs),
    GetClientStateUpdate(GetClientStateUpdateArgs),
    GetCheckpoint(GetCheckpointArgs),
    GetCheckpointsSummary(GetCheckpointsSummaryArgs),
    GetEpochSummary(GetEpochSummaryArgs),
    GetSyncinfo(GetSyncinfoArgs),
    GetChainstate(GetChainstateArgs),
    RevertChainstate(RevertChainstateArgs),
}

/// Output format
#[derive(PartialEq, Eq, Debug, Clone)]
pub(crate) enum OutputFormat {
    /// Machine-readable, concise format (default)
    Porcelain,
    /// Structured JSON
    Json,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct UnsupportedOutputFormat;

impl std::fmt::Display for UnsupportedOutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "must be 'porcelain' or 'json'")
    }
}

impl FromStr for OutputFormat {
    type Err = UnsupportedOutputFormat;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "porcelain" | "default" => Ok(Self::Porcelain),
            "json" => Ok(Self::Json),
            _ => Err(UnsupportedOutputFormat),
        }
    }
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            OutputFormat::Porcelain => "porcelain",
            OutputFormat::Json => "json",
        })
    }
}
