//! Types for chunked envelope publication.

use bitcoin::{hashes::Hash, Txid, Wtxid};
use strata_primitives::buf::Buf32;

use super::ChunkedEnvelopeError;

/// Maximum chunk payload size in bytes.
///
/// Chosen to keep reveal transactions well under the 400KB standardness limit
/// after accounting for envelope overhead (script structure, signature, control block).
pub const MAX_CHUNK_PAYLOAD: usize = 330_000;

/// Maximum total payload size (10MB).
///
/// Sanity limit to prevent memory issues and unreasonable costs.
/// 10MB ≈ 31 chunks ≈ $300+ at 10 sat/vB.
pub const MAX_PAYLOAD_SIZE: usize = 10 * 1024 * 1024;

/// Maximum number of chunks per payload.
///
/// Derived from MAX_PAYLOAD_SIZE / MAX_CHUNK_PAYLOAD, rounded up.
pub const MAX_CHUNKS: u16 = 31;

/// Default maximum retries for failed chunk publications.
pub const DEFAULT_MAX_RETRIES: u8 = 3;

/// Intent to publish a payload that may require chunking.
///
/// The payload is split into chunks of at most [`MAX_CHUNK_PAYLOAD`] bytes,
/// and each chunk is published as a separate reveal transaction.
#[derive(Clone, Debug)]
pub struct ChunkedPayloadIntent {
    /// Caller-provided identifier for tracking.
    id: Buf32,
    /// The full payload bytes.
    payload: Vec<u8>,
    /// OP_RETURN tag bytes (4 bytes). Also used in witness envelope.
    op_return_tag: [u8; 4],
    /// Optional: wtxid of the last chunk from a previous payload (for cross-payload linking).
    prev_tail_wtxid: Option<Wtxid>,
}

impl ChunkedPayloadIntent {
    /// Creates a new chunked payload intent with validation.
    pub fn new(
        id: Buf32,
        payload: Vec<u8>,
        op_return_tag: [u8; 4],
    ) -> Result<Self, ChunkedEnvelopeError> {
        if payload.len() > MAX_PAYLOAD_SIZE {
            return Err(ChunkedEnvelopeError::PayloadTooLarge {
                size: payload.len(),
                max: MAX_PAYLOAD_SIZE,
            });
        }
        Ok(Self {
            id,
            payload,
            op_return_tag,
            prev_tail_wtxid: None,
        })
    }

    /// Sets the previous tail wtxid for cross-payload linking.
    pub fn with_prev_tail_wtxid(mut self, wtxid: Wtxid) -> Self {
        self.prev_tail_wtxid = Some(wtxid);
        self
    }

    /// Returns the caller-provided identifier.
    pub fn id(&self) -> &Buf32 {
        &self.id
    }

    /// Returns the payload bytes.
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Returns the OP_RETURN tag bytes.
    pub fn op_return_tag(&self) -> [u8; 4] {
        self.op_return_tag
    }

    /// Returns the previous tail wtxid for cross-payload linking.
    pub fn prev_tail_wtxid(&self) -> Option<&Wtxid> {
        self.prev_tail_wtxid.as_ref()
    }

    /// Computes the payload hash (sha256).
    pub fn compute_payload_hash(&self) -> Buf32 {
        use bitcoin::hashes::sha256;
        let hash = sha256::Hash::hash(&self.payload);
        Buf32(hash.to_byte_array())
    }

    /// Returns the number of chunks this payload will be split into.
    pub fn chunk_count(&self) -> u16 {
        if self.payload.is_empty() {
            return 1; // Empty payload still needs 1 chunk
        }
        self.payload.len().div_ceil(MAX_CHUNK_PAYLOAD) as u16
    }

    /// Splits the payload into chunks.
    pub fn split_into_chunks(&self) -> Vec<&[u8]> {
        if self.payload.is_empty() {
            return vec![&[]];
        }
        self.payload.chunks(MAX_CHUNK_PAYLOAD).collect()
    }
}

/// Chunk header embedded in reveal script (affects taproot address).
///
/// This header is part of the taproot script. The `prev_chunk_wtxid` is NOT
/// included here - it goes in the OP_RETURN output to avoid circular dependencies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkedEnvelopeHeader {
    /// Version byte (0 for v0).
    version: u8,
    /// SHA256 hash of the full payload.
    payload_hash: [u8; 32],
    /// Zero-based index of this chunk.
    chunk_index: u16,
    /// Total number of chunks in the payload.
    total_chunks: u16,
}

impl ChunkedEnvelopeHeader {
    /// Size of the serialized header in bytes.
    pub const SIZE: usize = 37;

    /// Creates a new chunk header with invariant validation.
    pub fn new(
        payload_hash: [u8; 32],
        chunk_index: u16,
        total_chunks: u16,
    ) -> Result<Self, &'static str> {
        if total_chunks == 0 {
            return Err("total_chunks must be >= 1");
        }
        if chunk_index >= total_chunks {
            return Err("chunk_index must be < total_chunks");
        }
        Ok(Self {
            version: 0,
            payload_hash,
            chunk_index,
            total_chunks,
        })
    }

    /// Returns the version byte.
    pub fn version(&self) -> u8 {
        self.version
    }

    /// Returns the payload hash.
    pub fn payload_hash(&self) -> &[u8; 32] {
        &self.payload_hash
    }

    /// Returns the chunk index.
    pub fn chunk_index(&self) -> u16 {
        self.chunk_index
    }

    /// Returns the total number of chunks.
    pub fn total_chunks(&self) -> u16 {
        self.total_chunks
    }

    /// Serializes the header to bytes.
    pub fn serialize(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0] = self.version;
        buf[1..33].copy_from_slice(&self.payload_hash);
        buf[33..35].copy_from_slice(&self.chunk_index.to_le_bytes());
        buf[35..37].copy_from_slice(&self.total_chunks.to_le_bytes());
        buf
    }

    /// Deserializes the header from bytes.
    pub fn deserialize(buf: &[u8]) -> Option<Self> {
        if buf.len() < Self::SIZE {
            return None;
        }
        let version = buf[0];
        let mut payload_hash = [0u8; 32];
        payload_hash.copy_from_slice(&buf[1..33]);
        let chunk_index = u16::from_le_bytes([buf[33], buf[34]]);
        let total_chunks = u16::from_le_bytes([buf[35], buf[36]]);
        Some(Self {
            version,
            payload_hash,
            chunk_index,
            total_chunks,
        })
    }
}

/// Result after successful chunked submission.
#[derive(Clone, Debug)]
pub struct ChunkedSubmissionResult {
    /// SHA256 hash of the payload.
    pub payload_hash: Buf32,
    /// Total number of chunks.
    pub total_chunks: u16,
    /// Wtxids of the reveal transactions (ordered by chunk index).
    pub chunk_wtxids: Vec<Wtxid>,
    /// Txid of the batched commit transaction.
    pub commit_txid: Txid,
}

/// Publication status exposed to sequencer (simple 3-state).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DaStatus {
    /// Submission in progress.
    Pending,
    /// All chunks confirmed in L1 blocks. DA is published.
    Published,
    /// Publication failed.
    Failed { reason: String },
}

/// Internal status for btcio watcher (detailed, stored in DB).
///
/// Status progression: Pending → CommitConfirmed → AllRevealsConfirmed → Finalized
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DaBlobStatus {
    /// Transactions constructed, commit not yet confirmed.
    Pending,
    /// Commit tx confirmed, reveals being processed.
    CommitConfirmed {
        /// Number of confirmed reveals.
        reveals_confirmed: u16,
    },
    /// All reveal txs have at least 1 confirmation.
    AllRevealsConfirmed,
    /// All transactions finalized (sufficient confirmation depth).
    Finalized,
    /// Failed, requires manual intervention.
    Failed(String),
}

impl DaBlobStatus {
    /// Converts internal status to sequencer-facing status.
    pub fn to_public(&self) -> DaStatus {
        match self {
            Self::Pending | Self::CommitConfirmed { .. } => DaStatus::Pending,
            Self::AllRevealsConfirmed | Self::Finalized => DaStatus::Published,
            Self::Failed(reason) => DaStatus::Failed {
                reason: reason.clone(),
            },
        }
    }

    /// Returns true if DA is considered "published" for sequencer.
    pub fn is_published(&self) -> bool {
        matches!(self, Self::AllRevealsConfirmed | Self::Finalized)
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// State Machine Types (aligned with feat/ee-da-chunking)
// ═══════════════════════════════════════════════════════════════════════════════

/// Per-chunk publication status with retry tracking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChunkPublishStatus {
    /// Chunk is waiting to be submitted.
    Pending {
        /// Number of times submission has been retried.
        retry_count: u8,
    },
    /// Chunk has been submitted, waiting for confirmation.
    Submitted {
        /// Commitment identifier (txid or similar).
        commitment: [u8; 32],
        /// Number of times submission has been retried.
        retry_count: u8,
    },
    /// Chunk reveal transaction is in mempool or has minimal confirmations.
    Published {
        /// Wtxid of the reveal transaction.
        reveal_wtxid: Wtxid,
    },
    /// Chunk has been confirmed in a block.
    Confirmed {
        /// Wtxid of the reveal transaction.
        reveal_wtxid: Wtxid,
        /// Block height at which the reveal was confirmed.
        block_height: u64,
    },
    /// Chunk publication failed after max retries.
    Failed {
        /// Error description.
        error: String,
        /// Number of retries attempted.
        retry_count: u8,
    },
}

impl ChunkPublishStatus {
    /// Returns true if this chunk is in a terminal state (confirmed or failed).
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Confirmed { .. } | Self::Failed { .. })
    }

    /// Returns true if this chunk has been confirmed.
    pub fn is_confirmed(&self) -> bool {
        matches!(self, Self::Confirmed { .. })
    }

    /// Returns the retry count for this chunk.
    pub fn retry_count(&self) -> u8 {
        match self {
            Self::Pending { retry_count } => *retry_count,
            Self::Submitted { retry_count, .. } => *retry_count,
            Self::Failed { retry_count, .. } => *retry_count,
            Self::Published { .. } | Self::Confirmed { .. } => 0,
        }
    }
}

/// State machine tracking overall chunked publication progress.
#[derive(Clone, Debug)]
pub struct ChunkedPublishingState {
    /// SHA256 hash of the full payload.
    payload_hash: [u8; 32],
    /// Total number of chunks.
    total_chunks: u16,
    /// Per-chunk status tracking.
    chunk_statuses: Vec<ChunkPublishStatus>,
    /// Maximum retry attempts per chunk.
    max_retries: u8,
}

impl ChunkedPublishingState {
    /// Creates a new publishing state for the given payload.
    pub fn new(payload_hash: [u8; 32], total_chunks: u16) -> Self {
        Self::with_max_retries(payload_hash, total_chunks, DEFAULT_MAX_RETRIES)
    }

    /// Creates a new publishing state with custom max retries.
    pub fn with_max_retries(payload_hash: [u8; 32], total_chunks: u16, max_retries: u8) -> Self {
        let chunk_statuses = (0..total_chunks)
            .map(|_| ChunkPublishStatus::Pending { retry_count: 0 })
            .collect();
        Self {
            payload_hash,
            total_chunks,
            chunk_statuses,
            max_retries,
        }
    }

    /// Returns the payload hash.
    pub fn payload_hash(&self) -> &[u8; 32] {
        &self.payload_hash
    }

    /// Returns the total number of chunks.
    pub fn total_chunks(&self) -> u16 {
        self.total_chunks
    }

    /// Returns the status of a specific chunk.
    pub fn chunk_status(&self, index: u16) -> Option<&ChunkPublishStatus> {
        self.chunk_statuses.get(index as usize)
    }

    /// Returns all chunk statuses.
    pub fn chunk_statuses(&self) -> &[ChunkPublishStatus] {
        &self.chunk_statuses
    }

    /// Updates the status of a specific chunk.
    pub fn update_chunk_status(&mut self, index: u16, status: ChunkPublishStatus) {
        if let Some(slot) = self.chunk_statuses.get_mut(index as usize) {
            *slot = status;
        }
    }

    /// Marks a chunk as submitted.
    pub fn mark_submitted(&mut self, index: u16, commitment: [u8; 32]) {
        if let Some(status) = self.chunk_statuses.get_mut(index as usize) {
            let retry_count = status.retry_count();
            *status = ChunkPublishStatus::Submitted {
                commitment,
                retry_count,
            };
        }
    }

    /// Marks a chunk as published (in mempool).
    pub fn mark_published(&mut self, index: u16, reveal_wtxid: Wtxid) {
        if let Some(status) = self.chunk_statuses.get_mut(index as usize) {
            *status = ChunkPublishStatus::Published { reveal_wtxid };
        }
    }

    /// Marks a chunk as confirmed.
    pub fn mark_confirmed(&mut self, index: u16, reveal_wtxid: Wtxid, block_height: u64) {
        if let Some(status) = self.chunk_statuses.get_mut(index as usize) {
            *status = ChunkPublishStatus::Confirmed {
                reveal_wtxid,
                block_height,
            };
        }
    }

    /// Marks a chunk as failed.
    pub fn mark_failed(&mut self, index: u16, error: String) {
        if let Some(status) = self.chunk_statuses.get_mut(index as usize) {
            let retry_count = status.retry_count();
            *status = ChunkPublishStatus::Failed { error, retry_count };
        }
    }

    /// Increments retry count for a chunk and returns whether retries are exhausted.
    pub fn increment_retry(&mut self, index: u16) -> bool {
        if let Some(status) = self.chunk_statuses.get_mut(index as usize) {
            let retry_count = status.retry_count() + 1;
            if retry_count > self.max_retries {
                return true; // Exhausted
            }
            *status = ChunkPublishStatus::Pending { retry_count };
        }
        false
    }

    /// Returns true if all chunks are confirmed.
    pub fn all_confirmed(&self) -> bool {
        self.chunk_statuses
            .iter()
            .all(|s| matches!(s, ChunkPublishStatus::Confirmed { .. }))
    }

    /// Returns true if any chunk has permanently failed.
    pub fn any_failed(&self) -> bool {
        self.chunk_statuses
            .iter()
            .any(|s| matches!(s, ChunkPublishStatus::Failed { .. }))
    }

    /// Returns the number of confirmed chunks.
    pub fn confirmed_count(&self) -> u16 {
        self.chunk_statuses
            .iter()
            .filter(|s| matches!(s, ChunkPublishStatus::Confirmed { .. }))
            .count() as u16
    }

    /// Returns progress as (confirmed, total).
    pub fn progress(&self) -> (u16, u16) {
        (self.confirmed_count(), self.total_chunks)
    }

    /// Returns the overall status derived from chunk statuses.
    pub fn overall_status(&self) -> DaBlobStatus {
        if self.any_failed() {
            return DaBlobStatus::Failed("one or more chunks failed".to_string());
        }
        if self.all_confirmed() {
            return DaBlobStatus::AllRevealsConfirmed;
        }
        let confirmed = self.confirmed_count();
        if confirmed > 0 {
            DaBlobStatus::CommitConfirmed {
                reveals_confirmed: confirmed,
            }
        } else {
            DaBlobStatus::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunked_envelope_header_roundtrip() {
        let header = ChunkedEnvelopeHeader::new([0x42u8; 32], 5, 10).unwrap();
        let serialized = header.serialize();
        let deserialized = ChunkedEnvelopeHeader::deserialize(&serialized).unwrap();
        assert_eq!(header, deserialized);
    }

    #[test]
    fn test_chunked_envelope_header_validation() {
        // Valid
        assert!(ChunkedEnvelopeHeader::new([0; 32], 0, 1).is_ok());
        assert!(ChunkedEnvelopeHeader::new([0; 32], 9, 10).is_ok());

        // Invalid: total_chunks == 0
        assert!(ChunkedEnvelopeHeader::new([0; 32], 0, 0).is_err());

        // Invalid: chunk_index >= total_chunks
        assert!(ChunkedEnvelopeHeader::new([0; 32], 10, 10).is_err());
        assert!(ChunkedEnvelopeHeader::new([0; 32], 1, 1).is_err());
    }

    #[test]
    fn test_chunk_count() {
        // Empty payload
        let intent = ChunkedPayloadIntent::new(Buf32::zero(), vec![], [0; 4]).unwrap();
        assert_eq!(intent.chunk_count(), 1);

        // Small payload (1 chunk)
        let intent = ChunkedPayloadIntent::new(Buf32::zero(), vec![0u8; 1000], [0; 4]).unwrap();
        assert_eq!(intent.chunk_count(), 1);

        // Exactly MAX_CHUNK_PAYLOAD (1 chunk)
        let intent =
            ChunkedPayloadIntent::new(Buf32::zero(), vec![0u8; MAX_CHUNK_PAYLOAD], [0; 4]).unwrap();
        assert_eq!(intent.chunk_count(), 1);

        // Just over MAX_CHUNK_PAYLOAD (2 chunks)
        let intent =
            ChunkedPayloadIntent::new(Buf32::zero(), vec![0u8; MAX_CHUNK_PAYLOAD + 1], [0; 4])
                .unwrap();
        assert_eq!(intent.chunk_count(), 2);

        // Large payload
        let intent = ChunkedPayloadIntent::new(
            Buf32::zero(),
            vec![0u8; MAX_CHUNK_PAYLOAD * 3 + 100],
            [0; 4],
        )
        .unwrap();
        assert_eq!(intent.chunk_count(), 4);
    }

    #[test]
    fn test_payload_too_large() {
        let result =
            ChunkedPayloadIntent::new(Buf32::zero(), vec![0u8; MAX_PAYLOAD_SIZE + 1], [0; 4]);
        assert!(matches!(
            result,
            Err(ChunkedEnvelopeError::PayloadTooLarge { .. })
        ));
    }

    #[test]
    fn test_da_blob_status_to_public() {
        assert_eq!(DaBlobStatus::Pending.to_public(), DaStatus::Pending);
        assert_eq!(
            DaBlobStatus::CommitConfirmed {
                reveals_confirmed: 2
            }
            .to_public(),
            DaStatus::Pending
        );
        assert_eq!(
            DaBlobStatus::AllRevealsConfirmed.to_public(),
            DaStatus::Published
        );
        assert_eq!(DaBlobStatus::Finalized.to_public(), DaStatus::Published);
        assert_eq!(
            DaBlobStatus::Failed("test".to_string()).to_public(),
            DaStatus::Failed {
                reason: "test".to_string()
            }
        );
    }

    #[test]
    fn test_publishing_state_progression() {
        let mut state = ChunkedPublishingState::new([0x42; 32], 3);

        assert_eq!(state.confirmed_count(), 0);
        assert!(!state.all_confirmed());
        assert!(!state.any_failed());
        assert_eq!(state.progress(), (0, 3));

        // Mark first chunk confirmed
        state.mark_confirmed(0, Wtxid::all_zeros(), 100);
        assert_eq!(state.confirmed_count(), 1);
        assert_eq!(state.progress(), (1, 3));

        // Mark all chunks confirmed
        state.mark_confirmed(1, Wtxid::all_zeros(), 101);
        state.mark_confirmed(2, Wtxid::all_zeros(), 102);
        assert!(state.all_confirmed());
        assert_eq!(
            state.overall_status(),
            DaBlobStatus::AllRevealsConfirmed
        );
    }

    #[test]
    fn test_publishing_state_retry() {
        let mut state = ChunkedPublishingState::with_max_retries([0x42; 32], 2, 2);

        // First retry
        assert!(!state.increment_retry(0));
        assert_eq!(state.chunk_status(0).unwrap().retry_count(), 1);

        // Second retry
        assert!(!state.increment_retry(0));
        assert_eq!(state.chunk_status(0).unwrap().retry_count(), 2);

        // Third retry - exhausted
        assert!(state.increment_retry(0));
    }

    #[test]
    fn test_publishing_state_failure() {
        let mut state = ChunkedPublishingState::new([0x42; 32], 2);

        state.mark_confirmed(0, Wtxid::all_zeros(), 100);
        state.mark_failed(1, "network error".to_string());

        assert!(state.any_failed());
        assert!(!state.all_confirmed());
        assert!(matches!(
            state.overall_status(),
            DaBlobStatus::Failed(_)
        ));
    }
}
