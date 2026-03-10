use strata_identifiers::{AccountId, Hash};
use strata_ol_chain_types::L2BlockId;
use strata_primitives::{epoch::EpochCommitment, l1::L1BlockId, L1Height};
use strata_storage_common::exec::OpsError;
use thiserror::Error;
use typed_sled::error::Error;

use crate::{
    chainstate::WriteBatchId,
    mmr_index::{LeafPos, NodePos},
};

/// Pure MMR algorithm errors - domain-specific, no storage concepts.
#[derive(Debug, Clone, Error)]
pub enum MmrError {
    #[error("MMR leaf {0} not found")]
    LeafNotFound(u64),

    #[error("invalid mmr range (start {start}, end {end})")]
    InvalidRange { start: u64, end: u64 },

    #[error("mmr index {pos} out of bounds (max {max_size})")]
    PositionOutOfBounds { pos: u64, max_size: u64 },
}
#[derive(Debug, Error, Clone)]
pub enum DbError {
    #[error("entry with idx does not exist")]
    NonExistentEntry,

    #[error("entry with idx already exists")]
    EntryAlreadyExists,

    #[error("tried to insert into {0} out-of-order index {1}")]
    OooInsert(&'static str, L1Height),

    /// (type, missing, start, end)
    #[error("missing {0} block {1} in range {2}..{3}")]
    MissingBlockInRange(&'static str, u64, u64, u64),

    #[error("missing L1 block body (id {0})")]
    MissingL1BlockManifest(L1BlockId),

    #[error("missing L1 block (height {0})")]
    MissingL1Block(L1Height),

    #[error("L1 canonical chain is empty")]
    L1CanonicalChainEmpty,

    #[error("OL canonical chain is empty")]
    OLCanonicalChainEmpty,

    #[error("Revert height {0} above chain tip height {1}")]
    L1InvalidRevertHeight(L1Height, L1Height),

    #[error("Block does not extend canonical chain tip")]
    L1InvalidNextBlock(L1Height, L1BlockId),

    #[error("missing L2 block (id {0})")]
    MissingL2Block(L2BlockId),

    #[error("missing L2 block (slot {0})")]
    MissingL2BlockHeight(u64),

    #[error("missing L2 state (slot {0})")]
    MissingL2State(u64),

    #[error("missing state instance")]
    MissingStateInstance,

    #[error("missing write batch (id {0})")]
    MissingWriteBatch(WriteBatchId),

    #[error("missing slot write batch (id {0})")]
    MissingSlotWriteBatch(L2BlockId),

    #[error("missing epoch write batch (id {0})")]
    MissingEpochWriteBatch(L2BlockId),

    #[error("not yet bootstrapped")]
    NotBootstrapped,

    #[error("tried to overwrite batch checkpoint at idx {0}")]
    OverwriteCheckpoint(u64),

    #[error("tried to overwrite consensus checkpoint at idx {0}")]
    OverwriteConsensusCheckpoint(u64),

    #[error("tried to overwrite state update at idx{0}. must purge in order to be replaced")]
    OverwriteStateUpdate(u64),

    #[error("tried to purge data more recently than allowed")]
    PurgeTooRecent,

    #[error("unknown state index {0}")]
    UnknownIdx(u64),

    #[error("tried to overwrite epoch {0:?}")]
    OverwriteEpoch(EpochCommitment),

    #[error("tried to revert to index {0} above current tip {1}")]
    RevertAboveCurrent(u64, u64),

    #[error("IO Error: {0}")]
    IoError(String),

    #[error("operation timed out")]
    TimedOut,

    #[error("operation aborted")]
    Aborted,

    #[error("invalid argument")]
    InvalidArgument,

    #[error("resource busy")]
    Busy,

    /// A database worker task failed in an way that could not be determined.
    #[error("worker task exited strangely")]
    WorkerFailedStrangely,

    /// This happens in a cache when we were a second call to a database entry after a primary one
    /// was started whose result we would use failed.  This is meant to be a transient error that
    /// typically could be retried, but the specifics depend on the underlying database semantics.
    #[error("failed to load a cache entry")]
    CacheLoadFail,

    #[error("codec: {0}")]
    CodecError(String),

    #[error("transaction: {0}")]
    TransactionError(String),

    #[error("not yet implemented")]
    Unimplemented,

    /// MMR leaf not found at index
    #[error("MMR leaf not found at index {0}")]
    MmrLeafNotFound(u64),

    /// MMR leaf not found at index for account
    #[error("MMR leaf not found at index {0} for account {1}")]
    MmrLeafNotFoundForAccount(u64, AccountId),

    /// MMR leaf hash mismatched expected hash at index.
    ///
    /// This variant is produced by storage-manager level validation logic.
    #[error("MMR leaf hash mismatch at index {idx} (expected {expected:?}, got {got:?})")]
    MmrLeafHashMismatch { idx: u64, expected: Hash, got: Hash },

    /// Requested leaf index is out of range for current leaf count.
    #[error("MMR index out of range (requested {requested}, cur {cur})")]
    MmrIndexOutOfRange { requested: u64, cur: u64 },

    /// MMR preimage payload not found at leaf position.
    #[error("MMR preimage payload not found at leaf position {0}")]
    MmrPayloadNotFound(LeafPos),

    /// Tree position is out of bounds for current MMR size.
    #[error("MMR pos out of bounds (pos {pos}, max {max})")]
    MmrPositionOutOfBounds { pos: u64, max: u64 },

    /// Invalid MMR index range
    #[error("Invalid MMR index range: {start}..{end}")]
    MmrInvalidRange { start: u64, end: u64 },

    /// MMR node not found at the given tree position.
    #[error("MMR node not found at position {0}")]
    MmrNodeNotFound(NodePos),

    /// MMR index batch precondition failed.
    #[error("MMR precondition failed for {mmr_id:?}: {detail}")]
    MmrPreconditionFailed { mmr_id: Vec<u8>, detail: String },

    /// Operation retried but failed all attempts.
    #[error("retries exhausted after {attempts} attempts: {last_error}")]
    RetriesExhausted {
        attempts: usize,
        last_error: Box<DbError>,
    },

    #[error("{0}")]
    Other(String),
}

impl From<anyhow::Error> for DbError {
    fn from(value: anyhow::Error) -> Self {
        Self::Other(value.to_string())
    }
}

impl From<Error> for DbError {
    fn from(value: Error) -> Self {
        Self::Other(format!("sled error: {value:?}"))
    }
}

impl From<OpsError> for DbError {
    fn from(value: OpsError) -> Self {
        match value {
            OpsError::WorkerFailedStrangely => DbError::WorkerFailedStrangely,
        }
    }
}

impl From<MmrError> for DbError {
    fn from(value: MmrError) -> Self {
        match value {
            MmrError::LeafNotFound(idx) => DbError::MmrLeafNotFound(idx),
            MmrError::InvalidRange { start, end } => DbError::MmrInvalidRange { start, end },
            MmrError::PositionOutOfBounds { pos, max_size } => {
                DbError::MmrPositionOutOfBounds { pos, max: max_size }
            }
        }
    }
}
