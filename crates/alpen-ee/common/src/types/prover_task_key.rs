//! Shared EE prover task-key encoding.

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

fn tagged_range_key(tag: u8, prev_block: Hash, last_block: Hash) -> Vec<u8> {
    let mut buf = Vec::with_capacity(RANGE_TASK_KEY_BYTES);
    buf.push(tag);
    let prev: [u8; 32] = prev_block.into();
    let last: [u8; 32] = last_block.into();
    buf.extend_from_slice(&prev);
    buf.extend_from_slice(&last);
    buf
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
}
