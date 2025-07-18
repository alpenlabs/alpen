use strata_primitives::epoch::EpochCommitment;
use strata_state::{id::L2BlockId, l1::L1BlockId};
use thiserror::Error;

use crate::chainstate::WriteBatchId;
#[derive(Debug, Error, Clone)]
pub enum DbError {
    #[error("entry with idx does not exist")]
    NonExistentEntry,

    #[error("entry with idx already exists")]
    EntryAlreadyExists,

    #[error("tried to insert into {0} out-of-order index {1}")]
    OooInsert(&'static str, u64),

    /// (type, missing, start, end)
    #[error("missing {0} block {1} in range {2}..{3}")]
    MissingBlockInRange(&'static str, u64, u64, u64),

    #[error("missing L1 block body (id {0})")]
    MissingL1BlockManifest(L1BlockId),

    #[error("missing L1 block (height {0})")]
    MissingL1Block(u64),

    #[error("L1 canonical chain is empty")]
    L1CanonicalChainEmpty,

    #[error("Revert height {0} above chain tip height {0}")]
    L1InvalidRevertHeight(u64, u64),

    #[error("Block does not extend canonical chain tip")]
    L1InvalidNextBlock(u64, L1BlockId),

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

    #[error("IO Error (rocksdb)")]
    IoError,

    #[error("operation timed out (rocksdb)")]
    TimedOut,

    #[error("operation aborted (rocksdb)")]
    Aborted,

    #[error("invalid argument (rocksdb)")]
    InvalidArgument,

    #[error("resource busy (rocksdb)")]
    Busy,

    /// A database worker task failed in an way that could not be determined.
    #[error("worked task exited strangely")]
    WorkerFailedStrangely,

    /// This happens in a cache when we were a second call to a database entry after a primary one
    /// was startedd whose result we would use failed.  This is meant to be a transient error that
    /// typically could be retried, but the specifics depend on the underlying database semantics.
    #[error("failed to load a cache entry")]
    CacheLoadFail,

    #[error("codec error {0}")]
    CodecError(String),

    #[error("transaction error {0}")]
    TransactionError(String),

    #[error("rocksdb {0}")]
    RocksDb(String),

    #[error("not yet implemented")]
    Unimplemented,

    #[error("{0}")]
    Other(String),
}

impl From<anyhow::Error> for DbError {
    fn from(value: anyhow::Error) -> Self {
        Self::Other(value.to_string())
    }
}
