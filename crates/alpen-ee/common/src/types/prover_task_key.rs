//! Shared EE prover task-key encoding.

use std::fmt;

use strata_acct_types::Hash;

use super::{batch::BatchId, chunk::ChunkId};

/// EE chunk-prover task-key tag.
pub const CHUNK_TASK_KEY_TAG: u8 = b'c';

/// EE acct-prover task-key tag.
pub const BATCH_TASK_KEY_TAG: u8 = b'a';

/// Tag byte plus the `(prev_block, last_block)` range.
pub const RANGE_TASK_KEY_BYTES: usize = 1 + 32 + 32;

/// Encodes an EE chunk-prover task key.
pub fn encode_chunk_task_key(chunk_id: ChunkId) -> Vec<u8> {
    tagged_range_key(
        CHUNK_TASK_KEY_TAG,
        chunk_id.prev_block(),
        chunk_id.last_block(),
    )
}

/// Encodes an EE acct-prover task key.
pub fn encode_batch_task_key(batch_id: BatchId) -> Vec<u8> {
    tagged_range_key(
        BATCH_TASK_KEY_TAG,
        batch_id.prev_block(),
        batch_id.last_block(),
    )
}

/// Decodes an EE chunk-prover task key.
pub fn decode_chunk_task_key(bytes: &[u8]) -> Result<ChunkId, ProverTaskKeyDecodeError> {
    let (prev_block, last_block) =
        decode_tagged_range_key(ProverTaskKeyKind::Chunk, CHUNK_TASK_KEY_TAG, bytes)?;
    Ok(ChunkId::from_parts(prev_block, last_block))
}

/// Decodes an EE acct-prover task key.
pub fn decode_batch_task_key(bytes: &[u8]) -> Result<BatchId, ProverTaskKeyDecodeError> {
    let (prev_block, last_block) =
        decode_tagged_range_key(ProverTaskKeyKind::Batch, BATCH_TASK_KEY_TAG, bytes)?;
    Ok(BatchId::from_parts(prev_block, last_block))
}

/// EE prover task-key kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProverTaskKeyKind {
    /// Chunk proof task key.
    Chunk,
    /// Account proof task key.
    Batch,
}

impl fmt::Display for ProverTaskKeyKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Chunk => f.write_str("ChunkTask"),
            Self::Batch => f.write_str("BatchTask"),
        }
    }
}

/// Error returned when decoding an EE prover task key.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProverTaskKeyDecodeError {
    /// The key length did not match the fixed tagged range-key size.
    #[error("invalid {kind} byte length: expected {expected}, got {actual}")]
    InvalidLength {
        /// Task-key kind being decoded.
        kind: ProverTaskKeyKind,
        /// Expected byte length.
        expected: usize,
        /// Actual byte length.
        actual: usize,
    },
    /// The tag byte did not match the expected task type.
    #[error("invalid {kind} tag byte: expected 0x{expected:02x}, got 0x{actual:02x}")]
    InvalidTag {
        /// Task-key kind being decoded.
        kind: ProverTaskKeyKind,
        /// Expected tag byte.
        expected: u8,
        /// Actual tag byte.
        actual: u8,
    },
}

fn tagged_range_key(tag: u8, prev_block: Hash, last_block: Hash) -> Vec<u8> {
    let mut buf = Vec::with_capacity(RANGE_TASK_KEY_BYTES);
    buf.push(tag);
    let prev: [u8; 32] = prev_block.into();
    let last: [u8; 32] = last_block.into();
    buf.extend_from_slice(&prev);
    buf.extend_from_slice(&last);
    buf
}

fn decode_tagged_range_key(
    kind: ProverTaskKeyKind,
    expected_tag: u8,
    bytes: &[u8],
) -> Result<(Hash, Hash), ProverTaskKeyDecodeError> {
    if bytes.len() != RANGE_TASK_KEY_BYTES {
        return Err(ProverTaskKeyDecodeError::InvalidLength {
            kind,
            expected: RANGE_TASK_KEY_BYTES,
            actual: bytes.len(),
        });
    }
    if bytes[0] != expected_tag {
        return Err(ProverTaskKeyDecodeError::InvalidTag {
            kind,
            expected: expected_tag,
            actual: bytes[0],
        });
    }

    let mut prev = [0u8; 32];
    let mut last = [0u8; 32];
    prev.copy_from_slice(&bytes[1..33]);
    last.copy_from_slice(&bytes[33..]);

    Ok((Hash::from(prev), Hash::from(last)))
}

#[cfg(test)]
mod tests {
    use strata_identifiers::Buf32;

    use super::*;

    fn test_hash(byte: u8) -> Hash {
        let mut bytes = [0u8; 32];
        bytes[31] = byte;
        Buf32(bytes)
    }

    #[test]
    fn chunk_and_batch_keys_are_tagged_ranges() {
        let prev = test_hash(1);
        let last = test_hash(2);

        let chunk_key = encode_chunk_task_key(ChunkId::from_parts(prev, last));
        assert_eq!(chunk_key.len(), RANGE_TASK_KEY_BYTES);
        assert_eq!(chunk_key[0], CHUNK_TASK_KEY_TAG);
        assert_eq!(&chunk_key[1..33], <[u8; 32]>::from(prev).as_slice());
        assert_eq!(&chunk_key[33..], <[u8; 32]>::from(last).as_slice());

        let acct_key = encode_batch_task_key(BatchId::from_parts(prev, last));
        assert_eq!(acct_key.len(), RANGE_TASK_KEY_BYTES);
        assert_eq!(acct_key[0], BATCH_TASK_KEY_TAG);
        assert_eq!(&acct_key[1..33], <[u8; 32]>::from(prev).as_slice());
        assert_eq!(&acct_key[33..], <[u8; 32]>::from(last).as_slice());
    }

    #[test]
    fn task_keys_roundtrip_through_shared_decoder() {
        let prev = test_hash(1);
        let last = test_hash(2);

        let chunk_id = ChunkId::from_parts(prev, last);
        let batch_id = BatchId::from_parts(prev, last);

        assert_eq!(
            decode_chunk_task_key(&encode_chunk_task_key(chunk_id)).unwrap(),
            chunk_id
        );
        assert_eq!(
            decode_batch_task_key(&encode_batch_task_key(batch_id)).unwrap(),
            batch_id
        );
    }

    #[test]
    fn task_key_decoder_rejects_wrong_tag() {
        let batch_id = BatchId::from_parts(test_hash(1), test_hash(2));
        let err = decode_chunk_task_key(&encode_batch_task_key(batch_id)).unwrap_err();

        assert_eq!(
            err,
            ProverTaskKeyDecodeError::InvalidTag {
                kind: ProverTaskKeyKind::Chunk,
                expected: CHUNK_TASK_KEY_TAG,
                actual: BATCH_TASK_KEY_TAG,
            }
        );
    }
}
