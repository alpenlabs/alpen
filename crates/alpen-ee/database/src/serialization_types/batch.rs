//! Database serialization types for Batch and Chunk storage.

use alpen_ee_common::{
    Batch, BatchId, BatchStatus, Chunk, ChunkId, ChunkStatus, L1DaBlockInfo, L1DaBlockRef, ProofId,
};
use bitcoin::{hashes::Hash as _, Txid, Wtxid};
use serde::{Deserialize, Serialize};
use strata_acct_types::Hash;
use strata_identifiers::{Buf32, L1BlockCommitment, L1BlockId, L1Height, WtxidsRoot};

/// Database representation of a (Txid, Wtxid) pair.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
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
///
/// As a table key this encodes as the raw 64-byte concatenation `prev_block ‖ last_block`
/// (see the `KeyCodec` impls in `sleddb::schema`); as a value it goes through serde.
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq, Deserialize, Serialize)]
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

impl DBBatchId {
    /// Builds an id from its two raw halves.
    pub(crate) fn from_raw_parts(prev_block: [u8; 32], last_block: [u8; 32]) -> Self {
        Self {
            prev_block,
            last_block,
        }
    }

    /// Returns the raw halves, in key order.
    pub(crate) fn raw_parts(&self) -> (&[u8; 32], &[u8; 32]) {
        (&self.prev_block, &self.last_block)
    }
}

/// Database representation of a Batch.
///
/// `idx` is not stored: it is the table key, so readers supply it when rebuilding the domain
/// type via [`DBBatch::into_batch`].
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct DBBatch {
    prev_block: [u8; 32],
    last_block: [u8; 32],
    last_blocknum: u64,
    inner_blocks: Vec<[u8; 32]>,
}

impl From<Batch> for DBBatch {
    fn from(value: Batch) -> Self {
        Self {
            prev_block: value.prev_block().into(),
            last_block: value.last_block().into(),
            last_blocknum: value.last_blocknum(),
            inner_blocks: value.inner_blocks().iter().map(|h| (*h).into()).collect(),
        }
    }
}

impl DBBatch {
    /// Rebuilds the domain batch, taking `idx` from the table key.
    ///
    /// Returns `Err` because `Batch::new` and `Batch::new_genesis_batch` already return
    /// `Result<Batch, &'static str>`, which is propagated directly here.
    fn into_batch(self, idx: u64) -> Result<Batch, &'static str> {
        let inner_blocks: Vec<Hash> = self.inner_blocks.into_iter().map(Hash::from).collect();

        if idx == 0 {
            Batch::new_genesis_batch(Hash::from(self.last_block), self.last_blocknum)
        } else {
            Batch::new(
                idx,
                Hash::from(self.prev_block),
                Hash::from(self.last_block),
                self.last_blocknum,
                inner_blocks,
            )
        }
    }
}

/// Database representation of L1DaBlockRef.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub(crate) struct DBL1DaBlockRef {
    /// Height of the L1 block.
    block_height: L1Height,

    // TODO(db-refactor-part-17): mirror field pending upstream Buf32 serde fix
    /// Id of the L1 block.
    block_id: [u8; 32],

    // TODO(db-refactor-part-17): mirror field pending upstream Buf32 serde fix
    /// Witness transaction Merkle root for the L1 block.
    wtxids_root: [u8; 32],

    /// This batch's DA txs in this L1 block as raw `(txid, wtxid)` pairs.
    txns: Vec<DBTxidPair>,
}

impl From<L1DaBlockRef> for DBL1DaBlockRef {
    fn from(value: L1DaBlockRef) -> Self {
        Self {
            block_height: value.block.commitment.height(),
            block_id: Buf32::from(*value.block.commitment.blkid()).into(),
            wtxids_root: value.block.wtxids_root().as_ref().to_owned(),
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
            block: L1DaBlockInfo::new(
                L1BlockCommitment::new(
                    value.block_height,
                    L1BlockId::from(Buf32::from(value.block_id)),
                ),
                WtxidsRoot::from(Buf32::from(value.wtxids_root)),
            ),
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
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DBBatchStatus {
    Genesis,
    Sealed,
    DaPending {
        envelope_idx: u64,
    },
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
            BatchStatus::DaPending { envelope_idx } => Self::DaPending { envelope_idx },
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
            DBBatchStatus::DaPending { envelope_idx } => Self::DaPending { envelope_idx },
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
// TODO(trey): split apart batch data and status
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
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

    /// Rebuilds the domain batch and status, taking `idx` from the table key.
    pub(crate) fn into_parts(self, idx: u64) -> Result<(Batch, BatchStatus), &'static str> {
        let batch = self.batch.into_batch(idx)?;
        let status = self.status.into();
        Ok((batch, status))
    }
}

/// Database representation of a ChunkId.
///
/// As a table key this encodes as the raw 64-byte concatenation `prev_block ‖ last_block`
/// (see the `KeyCodec` impls in `sleddb::schema`); as a value it goes through serde.
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq, Deserialize, Serialize)]
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

impl DBChunkId {
    /// Builds an id from its two raw halves.
    pub(crate) fn from_raw_parts(prev_block: [u8; 32], last_block: [u8; 32]) -> Self {
        Self {
            prev_block,
            last_block,
        }
    }

    /// Returns the raw halves, in key order.
    pub(crate) fn raw_parts(&self) -> (&[u8; 32], &[u8; 32]) {
        (&self.prev_block, &self.last_block)
    }
}

/// Database representation of a Chunk.
///
/// `idx` is not stored: it is the table key, so readers supply it when rebuilding the domain
/// type via [`DBChunk::into_chunk`]. `batch_idx` is kept -- it is a real field, not the key.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub(crate) struct DBChunk {
    prev_block: [u8; 32],
    last_block: [u8; 32],
    last_blocknum: u64,
    batch_idx: u64,
    inner_blocks: Vec<[u8; 32]>,
}

impl From<Chunk> for DBChunk {
    fn from(value: Chunk) -> Self {
        Self {
            prev_block: value.prev_block().into(),
            last_block: value.last_block().into(),
            last_blocknum: value.last_blocknum(),
            batch_idx: value.batch_idx(),
            inner_blocks: value.inner_blocks().iter().map(|h| (*h).into()).collect(),
        }
    }
}

impl DBChunk {
    /// Rebuilds the domain chunk, taking `idx` from the table key.
    fn into_chunk(self, idx: u64) -> Chunk {
        let inner_blocks: Vec<Hash> = self.inner_blocks.into_iter().map(Hash::from).collect();
        Chunk::new(
            idx,
            Hash::from(self.prev_block),
            Hash::from(self.last_block),
            self.last_blocknum,
            self.batch_idx,
            inner_blocks,
        )
    }
}

/// Database representation of ChunkStatus.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
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
// TODO(trey): split apart chunk and status
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
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

    /// Rebuilds the domain chunk and status, taking `idx` from the table key.
    pub(crate) fn into_parts(self, idx: u64) -> (Chunk, ChunkStatus) {
        let chunk = self.chunk.into_chunk(idx);
        let status = self.status.into();
        (chunk, status)
    }
}
