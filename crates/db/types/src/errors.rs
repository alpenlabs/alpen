use strata_identifiers::{AccountId, Epoch, Hash, OLBlockCommitment, OLBlockId, Slot};
use strata_primitives::epoch::EpochCommitment;
use strata_primitives::L1Height;
use strata_storage_common::exec::OpsError;
use thiserror::Error;
#[cfg(feature = "proxies")]
use tokio::task::JoinError;

use crate::mmr_index::{LeafPos, NodePos};

#[derive(Clone, Debug, Error)]
pub enum DbError {
    #[error("entry with idx does not exist")]
    NonExistentEntry,

    #[error("entry with idx already exists")]
    EntryAlreadyExists,

    #[error("tried to insert into {0} out-of-order index {1}")]
    OooInsert(&'static str, L1Height),

    #[error("L1 canonical chain is empty")]
    L1CanonicalChainEmpty,

    #[error("Revert height {0} above chain tip height {1}")]
    L1InvalidRevertHeight(L1Height, L1Height),

    #[error("not yet bootstrapped")]
    NotBootstrapped,

    #[error("tried to overwrite epoch {0:?}")]
    OverwriteEpoch(EpochCommitment),

    #[error("invalid argument")]
    InvalidArgument,

    #[error(
        "OL canonical suffix from slot {start_slot} with {block_count} blocks overflows slot range"
    )]
    OLCanonicalSuffixOverflow {
        start_slot: Slot,
        block_count: usize,
    },

    /// A terminal header was stored under a key other than its computed block ID.
    #[error("terminal OL header block ID mismatch: key {key}, computed {computed}")]
    OLTerminalHeaderIdMismatch { key: OLBlockId, computed: OLBlockId },

    /// Promotion attempted to replace an already-established history base.
    #[error("OL history base conflict: attempted {attempted}, current {current}")]
    OLHistoryBaseConflict {
        attempted: EpochCommitment,
        current: EpochCommitment,
    },

    #[error("resource busy")]
    Busy,

    /// A database worker task did not return a result.
    ///
    /// Produced when a blocking database task panics or is cancelled (its
    /// [`tokio::task::JoinError`] is stringified into the payload), or when a
    /// worker drops its response channel before sending. The payload describes
    /// the underlying failure.
    #[error("worker task failed to return a result: {0}")]
    WorkerFailedStrangely(String),

    #[error("codec: {0}")]
    CodecError(String),

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
    #[error("MMR preimage payload not found at leaf position {0:?}")]
    MmrPayloadNotFound(LeafPos),

    /// Tree position is out of bounds for current MMR size.
    #[error("MMR pos out of bounds (pos {pos}, max {max})")]
    MmrPositionOutOfBounds { pos: u64, max: u64 },

    /// Invalid MMR index range
    #[error("Invalid MMR index range: {start}..{end}")]
    MmrInvalidRange { start: u64, end: u64 },

    /// MMR node not found at the given tree position.
    #[error("MMR node not found at position {0:?}")]
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

    /// `apply_block_indexing` was called for a block whose slot does not
    /// strictly advance past the last applied block for this epoch.
    /// `attempted` is the incoming block; `last_applied` is what was already
    /// recorded for this epoch.
    #[error(
        "block indexing conflict for epoch {epoch}: \
         attempted {attempted}, last applied {last_applied}"
    )]
    BlockIndexingConflict {
        epoch: Epoch,
        attempted: OLBlockCommitment,
        last_applied: OLBlockCommitment,
    },

    /// `put_block_data_with_high_watermark` was called for a block whose slot does not strictly
    /// advance past the current high-watermark.
    #[error("block high-watermark conflict: attempted {attempted}, current {current}")]
    BlockHighWatermarkConflict {
        attempted: OLBlockCommitment,
        current: OLBlockCommitment,
    },

    #[error("{0}")]
    Other(String),
}

// TODO(STR-4241): this conversion is inverted -- the ops layer should map into
// `DbError`, not the other way around. Part 10 owns the move.
impl From<OpsError> for DbError {
    fn from(value: OpsError) -> Self {
        match value {
            OpsError::WorkerFailedStrangely => DbError::WorkerFailedStrangely(value.to_string()),
        }
    }
}

#[cfg(feature = "proxies")]
impl From<JoinError> for DbError {
    fn from(err: JoinError) -> Self {
        DbError::WorkerFailedStrangely(err.to_string())
    }
}
