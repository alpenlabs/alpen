//! Database serialization types for Batch and Chunk storage.

use alpen_ee_common::{
    Batch, BatchId, BatchStatus, Chunk, ChunkId, ChunkStatus, L1DaBlockRef, ProofId,
};
use bitcoin::{hashes::Hash as _, Txid, Wtxid};
use borsh::{BorshDeserialize, BorshSerialize};
use strata_acct_types::Hash;
use strata_identifiers::L1BlockCommitment;

/// Database representation of a (Txid, Wtxid) pair.
///
/// Uses named fields to avoid confusion between the two identically-typed 32-byte arrays.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
pub(crate) struct DBTxidPair {
    txid: [u8; 32],
    wtxid: [u8; 32],
}

impl DBTxidPair {
    fn new(txid: [u8; 32], wtxid: [u8; 32]) -> Self {
        Self { txid, wtxid }
    }

    fn into_parts(self) -> ([u8; 32], [u8; 32]) {
        (self.txid, self.wtxid)
    }
}

/// Database representation of a BatchId.
#[derive(Debug, Clone, PartialEq, Eq, Hash, BorshSerialize, BorshDeserialize)]
pub(crate) struct DBBatchId {
    prev_block: [u8; 32],
    last_block: [u8; 32],
}

impl From<BatchId> for DBBatchId {
    fn from(value: BatchId) -> Self {
        Self {
            prev_block: value.prev_block().into(),
            last_block: value.last_block().into(),
        }
    }
}

impl From<DBBatchId> for BatchId {
    fn from(value: DBBatchId) -> Self {
        BatchId::from_parts(Hash::from(value.prev_block), Hash::from(value.last_block))
    }
}

/// Database representation of a Batch.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
pub(crate) struct DBBatch {
    idx: u64,
    prev_block: [u8; 32],
    last_block: [u8; 32],
    last_blocknum: u64,
    inner_blocks: Vec<[u8; 32]>,
}

impl From<Batch> for DBBatch {
    fn from(value: Batch) -> Self {
        Self {
            idx: value.idx(),
            prev_block: value.prev_block().into(),
            last_block: value.last_block().into(),
            last_blocknum: value.last_blocknum(),
            inner_blocks: value.inner_blocks().iter().map(|h| (*h).into()).collect(),
        }
    }
}

impl TryFrom<DBBatch> for Batch {
    type Error = &'static str;

    /// Converts a database batch into a domain batch.
    ///
    /// Note: The return type is `Result` because `Batch::new` and `Batch::new_genesis_batch`
    /// already return `Result<Batch, &'static str>`, which is propagated directly here.
    fn try_from(value: DBBatch) -> Result<Self, Self::Error> {
        let inner_blocks: Vec<Hash> = value.inner_blocks.into_iter().map(Hash::from).collect();

        if value.idx == 0 {
            Batch::new_genesis_batch(Hash::from(value.last_block), value.last_blocknum)
        } else {
            Batch::new(
                value.idx,
                Hash::from(value.prev_block),
                Hash::from(value.last_block),
                value.last_blocknum,
                inner_blocks,
            )
        }
    }
}

/// Database representation of L1DaBlockRef.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
pub(crate) struct DBL1DaBlockRef {
    /// L1BlockCommitment serialized via its Borsh impl.
    block: L1BlockCommitment,
    /// Transactions as (txid, wtxid) pairs, stored as raw bytes.
    txns: Vec<DBTxidPair>,
}

impl From<L1DaBlockRef> for DBL1DaBlockRef {
    fn from(value: L1DaBlockRef) -> Self {
        Self {
            block: value.block,
            txns: value
                .txns
                .into_iter()
                .map(|(txid, wtxid)| DBTxidPair::new(txid.to_byte_array(), wtxid.to_byte_array()))
                .collect(),
        }
    }
}

impl From<DBL1DaBlockRef> for L1DaBlockRef {
    fn from(value: DBL1DaBlockRef) -> Self {
        Self {
            block: value.block,
            txns: value
                .txns
                .into_iter()
                .map(|pair| {
                    let (txid_bytes, wtxid_bytes) = pair.into_parts();
                    (
                        Txid::from_byte_array(txid_bytes),
                        Wtxid::from_byte_array(wtxid_bytes),
                    )
                })
                .collect(),
        }
    }
}

/// Database representation of BatchStatus.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
pub(crate) enum DBBatchStatus {
    Genesis,
    Sealed,
    DaPending,
    DaComplete {
        da: Vec<DBL1DaBlockRef>,
    },
    ProofPending {
        da: Vec<DBL1DaBlockRef>,
    },
    ProofReady {
        da: Vec<DBL1DaBlockRef>,
        proof: [u8; 32],
    },
}

impl From<BatchStatus> for DBBatchStatus {
    fn from(value: BatchStatus) -> Self {
        match value {
            BatchStatus::Genesis => Self::Genesis,
            BatchStatus::Sealed => Self::Sealed,
            BatchStatus::DaPending => Self::DaPending,
            BatchStatus::DaComplete { da } => Self::DaComplete {
                da: da.into_iter().map(Into::into).collect(),
            },
            BatchStatus::ProofPending { da } => Self::ProofPending {
                da: da.into_iter().map(Into::into).collect(),
            },
            BatchStatus::ProofReady { da, proof } => Self::ProofReady {
                da: da.into_iter().map(Into::into).collect(),
                proof: proof.into(),
            },
        }
    }
}

impl From<DBBatchStatus> for BatchStatus {
    fn from(value: DBBatchStatus) -> Self {
        match value {
            DBBatchStatus::Genesis => Self::Genesis,
            DBBatchStatus::Sealed => Self::Sealed,
            DBBatchStatus::DaPending => Self::DaPending,
            DBBatchStatus::DaComplete { da } => Self::DaComplete {
                da: da.into_iter().map(Into::into).collect(),
            },
            DBBatchStatus::ProofPending { da } => Self::ProofPending {
                da: da.into_iter().map(Into::into).collect(),
            },
            DBBatchStatus::ProofReady { da, proof } => Self::ProofReady {
                da: da.into_iter().map(Into::into).collect(),
                proof: ProofId::from(proof),
            },
        }
    }
}

/// Database representation of a Batch with its status, stored together.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
pub(crate) struct DBBatchWithStatus {
    batch: DBBatch,
    status: DBBatchStatus,
}

impl DBBatchWithStatus {
    pub(crate) fn new(batch: Batch, status: BatchStatus) -> Self {
        Self {
            batch: batch.into(),
            status: status.into(),
        }
    }

    pub(crate) fn into_parts(self) -> Result<(Batch, BatchStatus), &'static str> {
        let batch = self.batch.try_into()?;
        let status = self.status.into();
        Ok((batch, status))
    }
}

/// Database representation of a ChunkId.
#[derive(Debug, Clone, PartialEq, Eq, Hash, BorshSerialize, BorshDeserialize)]
pub(crate) struct DBChunkId {
    prev_block: [u8; 32],
    last_block: [u8; 32],
}

impl From<ChunkId> for DBChunkId {
    fn from(value: ChunkId) -> Self {
        Self {
            prev_block: value.prev_block().into(),
            last_block: value.last_block().into(),
        }
    }
}

impl From<DBChunkId> for ChunkId {
    fn from(value: DBChunkId) -> Self {
        ChunkId::from_parts(Hash::from(value.prev_block), Hash::from(value.last_block))
    }
}

/// Database representation of a Chunk.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
pub(crate) struct DBChunk {
    idx: u64,
    prev_block: [u8; 32],
    last_block: [u8; 32],
    inner_blocks: Vec<[u8; 32]>,
}

impl From<Chunk> for DBChunk {
    fn from(value: Chunk) -> Self {
        Self {
            idx: value.idx(),
            prev_block: value.prev_block().into(),
            last_block: value.last_block().into(),
            inner_blocks: value.inner_blocks().iter().map(|h| (*h).into()).collect(),
        }
    }
}

impl From<DBChunk> for Chunk {
    fn from(value: DBChunk) -> Self {
        let inner_blocks: Vec<Hash> = value.inner_blocks.into_iter().map(Hash::from).collect();
        Chunk::new(
            value.idx,
            Hash::from(value.prev_block),
            Hash::from(value.last_block),
            inner_blocks,
        )
    }
}

/// Database representation of ChunkStatus.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
pub(crate) enum DBChunkStatus {
    ProvingNotStarted,
    ProofPending(String),
    ProofReady([u8; 32]),
}

impl From<ChunkStatus> for DBChunkStatus {
    fn from(value: ChunkStatus) -> Self {
        match value {
            ChunkStatus::ProvingNotStarted => Self::ProvingNotStarted,
            ChunkStatus::ProofPending(s) => Self::ProofPending(s),
            ChunkStatus::ProofReady(proof) => Self::ProofReady(proof.into()),
        }
    }
}

impl From<DBChunkStatus> for ChunkStatus {
    fn from(value: DBChunkStatus) -> Self {
        match value {
            DBChunkStatus::ProvingNotStarted => Self::ProvingNotStarted,
            DBChunkStatus::ProofPending(s) => Self::ProofPending(s),
            DBChunkStatus::ProofReady(proof) => Self::ProofReady(ProofId::from(proof)),
        }
    }
}

/// Database representation of a Chunk with its status, stored together.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
pub(crate) struct DBChunkWithStatus {
    chunk: DBChunk,
    status: DBChunkStatus,
}

impl DBChunkWithStatus {
    pub(crate) fn new(chunk: Chunk, status: ChunkStatus) -> Self {
        Self {
            chunk: chunk.into(),
            status: status.into(),
        }
    }

    pub(crate) fn into_parts(self) -> (Chunk, ChunkStatus) {
        let chunk = self.chunk.into();
        let status = self.status.into();
        (chunk, status)
    }
}
