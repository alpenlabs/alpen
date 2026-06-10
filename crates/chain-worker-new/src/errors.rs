//! Error types for the chain worker.

use strata_acct_types::AccountSerial;
use strata_codec::CodecError;
use strata_db_types::errors::DbError;
use strata_identifiers::{AccountId, Buf32, Epoch, OLBlockCommitment, OLBlockId};
use strata_ledger_types::StateError;
use strata_primitives::epoch::EpochCommitment;
use strata_snark_acct_types::Seqno;
use thiserror::Error;

/// Return type for worker messages.
pub type WorkerResult<T> = Result<T, WorkerError>;

/// Errors that can occur during chain worker operations.
#[derive(Debug, Error)]
pub enum WorkerError {
    /// Block not found in database.
    #[error("missing OL block {0}")]
    MissingOLBlock(OLBlockId),

    /// This usually means that we didn't execute the previous block.
    #[error("missing pre-state to execute block {0:?}")]
    MissingPreState(OLBlockCommitment),

    /// This might point to a database corruption or misused admin commands.
    /// The worker should not have tried to access block outputs that are missing.
    #[error("missing exec output for block {0:?}")]
    MissingBlockOutput(OLBlockCommitment),

    /// Missing write batch for a specific block, used for post-state reconstruction.
    #[error("missing write batch for block {0:?}")]
    MissingWriteBatch(OLBlockCommitment),

    /// This means that we haven't executed the block that's the terminal for an epoch.
    #[error("missing inner post-state of epoch {0} terminal {1:?}")]
    MissingEpochInnerPostState(u64, OLBlockCommitment),

    /// Missing epoch summary for the given commitment.
    #[error("missing summary for epoch commitment {0:?}")]
    MissingEpochSummary(EpochCommitment),

    /// Missing epoch summary for given epoch.
    #[error("missing summary for epoch {0:?}")]
    MissingSummaryForEpoch(u32),

    /// Missing checkpoint payload for an epoch the worker was asked to apply.
    #[error("missing checkpoint payload for epoch {0:?}")]
    MissingCheckpointPayload(EpochCommitment),

    /// Checkpoint sync was asked to apply epoch 0, which is always
    /// genesis-initialized locally and never reconstructed from a checkpoint.
    #[error("cannot apply checkpoint for epoch 0 (genesis-initialized)")]
    CannotApplyGenesisEpoch,

    /// The checkpoint payload's tip does not match the epoch being applied.
    /// Indicates a key→value mismatch from a storage-writer bug.
    #[error(
        "checkpoint payload tip mismatch at epoch {epoch} (payload epoch {payload_epoch}, payload l2 {payload_l2:?})"
    )]
    CheckpointTipMismatch {
        epoch: Epoch,
        payload_epoch: u32,
        payload_l2: OLBlockCommitment,
    },

    /// The L1 range derived from the base state and payload tip is inverted.
    #[error("checkpoint payload L1 range inverted at epoch {epoch} (from {from}, to {to})")]
    L1RangeInverted { epoch: Epoch, from: u32, to: u32 },

    /// The L1 manifest range exceeds the epoch manifest limit.
    #[error("epoch L1 manifest range too large at epoch {epoch} ({len} blocks, max {max})")]
    L1RangeTooLarge { epoch: Epoch, len: u32, max: u32 },

    /// Failed to decode the OL DA payload from the checkpoint sidecar.
    #[error("decode OL DA payload at epoch {epoch}: {source}")]
    DaPayloadDecode {
        epoch: Epoch,
        #[source]
        source: CodecError,
    },

    /// Failed to compute a state root during reconstruction.
    /// `stage` discriminates the call site (e.g. "indexer", "final").
    #[error("compute state root at epoch {epoch} ({stage}): {source}")]
    StateRootCompute {
        epoch: Epoch,
        stage: &'static str,
        #[source]
        source: StateError,
    },

    /// The indexer-derived state root and the post-batch state root diverge.
    #[error("state root divergence at epoch {epoch} (indexer {indexer_root}, batch {final_root})")]
    StateRootDivergence {
        epoch: Epoch,
        indexer_root: Buf32,
        final_root: Buf32,
    },

    /// The terminal block id reconstructed from the sidecar header complement
    /// does not match the epoch's terminal commitment.
    #[error(
        "terminal blkid mismatch at epoch {epoch} (expected {expected:?}, reconstructed {reconstructed:?})"
    )]
    TerminalBlkidMismatch {
        epoch: Epoch,
        expected: OLBlockId,
        reconstructed: OLBlockId,
    },

    /// A checkpoint log references an account serial unknown to the post-state.
    #[error("snark log references unknown account serial {0:?}")]
    UnknownAccountSerial(AccountSerial),

    /// A snark account's post-batch seqno differs from the seqno derived from
    /// per-update logs.
    #[error(
        "snark seqno mismatch for {serial:?} ({account_id}, derived {derived:?}, post {post:?})"
    )]
    SnarkSeqnoMismatch {
        serial: AccountSerial,
        account_id: AccountId,
        derived: Seqno,
        post: Seqno,
    },

    /// Failed to read account state during checkpoint reconstruction.
    /// `stage` discriminates pre/post-state contexts.
    #[error("read account state at {stage}: {source}")]
    AccountStateRead {
        stage: &'static str,
        #[source]
        source: StateError,
    },

    /// Generated by the worker handle when the worker has exited before being
    /// able to process a message we were trying to send.
    #[error("chain worker exited")]
    WorkerExited,

    /// STF execution error.
    #[error("STF execution failure: {0}")]
    StfExecution(#[from] strata_ol_stf::ExecError),

    /// Write-batch application failed when committing executed-block state.
    #[error("apply_write_batch failed at {commitment:?}: {source}")]
    ApplyWriteBatch {
        commitment: OLBlockCommitment,
        #[source]
        source: StateError,
    },

    /// Database error.
    #[error("database failure: {0}")]
    Database(#[from] DbError),

    /// A log payload was not a well-formed msg-fmt envelope.
    #[error("malformed OL log envelope: {0}")]
    MalformedLogEnvelope(#[from] strata_msg_fmt::Error),

    /// A snark-account update log failed to decode while sourcing index `extra_data`.
    #[error("failed to decode snark-account update log: {0}")]
    SnarkUpdateLogDecode(#[from] strata_ol_chain_types_new::LogDecodeError),

    /// The emitted snark-account update logs could not be paired 1:1 with the tracked snark
    /// state updates when sourcing index `extra_data`.
    #[error("snark update log/index count mismatch (expected {expected}, got {found})")]
    SnarkUpdateLogCountMismatch { expected: usize, found: usize },

    /// A snark-account update log did not line up with its tracked state update.
    #[error("snark update log next_read_idx mismatch (expected {expected}, got {found})")]
    SnarkUpdateLogMismatch { expected: u64, found: u64 },

    /// Missing a required dependency for operation.
    #[error("missing required dependency: {0}")]
    MissingDependency(&'static str),

    /// Worker shutdown before genesis was processed.
    #[error("shutdown before genesis")]
    ShutdownBeforeGenesis,

    /// Genesis block not found at slot 0.
    #[error("genesis block not found at slot 0")]
    MissingGenesisBlock,

    /// Worker has not been initialized yet.
    #[error("worker not initialized")]
    NotInitialized,

    /// Generic unexpected error.
    #[error("unexpected failure: {0}")]
    Unexpected(String),

    /// Functionality not yet implemented.
    #[error("not yet implemented")]
    Unimplemented,
}
