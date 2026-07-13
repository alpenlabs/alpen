use std::{
    fmt::{self, Display},
    path::PathBuf,
    str::FromStr,
};

use argh::FromArgs;

use crate::cmd::{
    broadcaster::{GetBroadcasterSummaryArgs, GetBroadcasterTxArgs},
    checkpoint::{GetCheckpointArgs, GetCheckpointsSummaryArgs, GetEpochSummaryArgs},
    checkpoint_proof::{DeleteCheckpointProofArgs, GetCheckpointProofArgs},
    client_state::GetClientStateUpdateArgs,
    ee_prover_task::{
        EeAbandonProverTaskArgs, EeAbandonProverTasksArgs, EeBackfillProverTaskRawArgs,
        EeDeleteProverTaskArgs, EeGetProverTaskArgs, EeGetProverTasksSummaryArgs,
        EeResetProverTaskArgs,
    },
    ee_receipts::{
        EeDeleteAcctProofArgs, EeDeleteChunkReceiptArgs, EeGetAcctProofArgs, EeGetChunkReceiptArgs,
    },
    ee_revert::EeRevertBatchesArgs,
    l1::{GetL1BlockArgs, GetL1SummaryArgs},
    mmr::{GetMmrLeafArgs, GetMmrSummaryArgs},
    ol::{DeleteOLBlockArgs, GetOLBlockArgs, GetOLBlocksAtSlotArgs, GetOLSummaryArgs},
    ol_state::{GetOLStateArgs, RevertOLStateArgs},
    prover_task::{
        AbandonProverTaskArgs, AbandonProverTasksArgs, BackfillCheckpointProofTaskArgs,
        BackfillProverTaskRawArgs, DeleteProverTaskArgs, GetProverTaskArgs,
        GetProverTasksSummaryArgs, ResetProverTaskArgs,
    },
    syncinfo::GetSyncinfoArgs,
    writer::{GetWriterPayloadArgs, GetWriterSummaryArgs},
};

/// Strata DB tool – offline database & chain‑maintenance utility.
#[derive(FromArgs)]
/// Inspect, repair and roll back an Strata node's database while the node is offline.
pub(crate) struct Cli {
    /// data directory of the node whose DB is being inspected. For
    /// `ee-*` subcommands, point this at the alpen-client's `--datadir`
    /// instead of the strata node's — each invocation is standalone and
    /// opens exactly one sled.
    #[argh(option, short = 'd', default = "PathBuf::from(\"data\")")]
    pub(crate) datadir: PathBuf,

    #[argh(subcommand)]
    pub(crate) cmd: Command,
}

/// Subcommand variants.
#[derive(FromArgs, Debug)]
#[argh(subcommand)]
pub(crate) enum Command {
    GetL1Block(GetL1BlockArgs),
    GetL1Summary(GetL1SummaryArgs),
    GetWriterSummary(GetWriterSummaryArgs),
    GetWriterPayload(GetWriterPayloadArgs),
    GetBroadcasterSummary(GetBroadcasterSummaryArgs),
    GetBroadcasterTx(GetBroadcasterTxArgs),
    GetOlBlock(GetOLBlockArgs),
    GetOlBlocksAtSlot(GetOLBlocksAtSlotArgs),
    GetOlSummary(GetOLSummaryArgs),
    DeleteOlBlock(DeleteOLBlockArgs),
    GetMmrSummary(GetMmrSummaryArgs),
    GetMmrLeaf(GetMmrLeafArgs),
    GetClientStateUpdate(GetClientStateUpdateArgs),
    GetCheckpoint(GetCheckpointArgs),
    GetCheckpointsSummary(GetCheckpointsSummaryArgs),
    GetEpochSummary(GetEpochSummaryArgs),
    GetSyncinfo(GetSyncinfoArgs),
    GetOLState(GetOLStateArgs),
    RevertOLState(RevertOLStateArgs),
    GetProverTask(GetProverTaskArgs),
    GetProverTasksSummary(GetProverTasksSummaryArgs),
    AbandonProverTask(AbandonProverTaskArgs),
    AbandonProverTasks(AbandonProverTasksArgs),
    ResetProverTask(ResetProverTaskArgs),
    DeleteProverTask(DeleteProverTaskArgs),
    GetCheckpointProof(GetCheckpointProofArgs),
    DeleteCheckpointProof(DeleteCheckpointProofArgs),
    BackfillCheckpointProofTask(BackfillCheckpointProofTaskArgs),
    BackfillProverTaskRaw(BackfillProverTaskRawArgs),
    EeGetProverTask(EeGetProverTaskArgs),
    EeGetProverTasksSummary(EeGetProverTasksSummaryArgs),
    EeAbandonProverTask(EeAbandonProverTaskArgs),
    EeAbandonProverTasks(EeAbandonProverTasksArgs),
    EeResetProverTask(EeResetProverTaskArgs),
    EeDeleteProverTask(EeDeleteProverTaskArgs),
    EeBackfillProverTaskRaw(EeBackfillProverTaskRawArgs),
    EeGetChunkReceipt(EeGetChunkReceiptArgs),
    EeDeleteChunkReceipt(EeDeleteChunkReceiptArgs),
    EeGetAcctProof(EeGetAcctProofArgs),
    EeDeleteAcctProof(EeDeleteAcctProofArgs),
    EeRevertBatches(EeRevertBatchesArgs),
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

impl Display for UnsupportedOutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

impl Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            OutputFormat::Porcelain => "porcelain",
            OutputFormat::Json => "json",
        })
    }
}
