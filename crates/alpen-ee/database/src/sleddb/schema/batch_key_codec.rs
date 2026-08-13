//! Key codecs for the batch/chunk identifier key types.
//!
//! Both encode as the raw 64-byte concatenation `prev_block ‖ last_block`. A serde-based key
//! codec would bloat the key and, worse, break the lexicographic ordering the tables rely on,
//! so these are written out by hand.

use typed_sled::{
    codec::{CodecError, KeyCodec},
    Schema,
};

use super::{
    AcctProofReceiptSchema, BatchChunksSchema, BatchIdToIdxSchema, ChunkIdToIdxSchema, DBBatchId,
    DBChunkId,
};

/// Width of one identifier half.
const HALF_LEN: usize = 32;

/// Total width of an encoded identifier.
const KEY_LEN: usize = HALF_LEN * 2;

/// Concatenates the two halves into the on-disk key form.
fn encode_halves(prev_block: &[u8; HALF_LEN], last_block: &[u8; HALF_LEN]) -> Vec<u8> {
    let mut out = Vec::with_capacity(KEY_LEN);
    out.extend_from_slice(prev_block);
    out.extend_from_slice(last_block);
    out
}

/// Splits an on-disk key back into its two halves.
fn decode_halves(
    data: &[u8],
    schema: &'static str,
) -> Result<([u8; HALF_LEN], [u8; HALF_LEN]), CodecError> {
    if data.len() != KEY_LEN {
        return Err(CodecError::InvalidKeyLength {
            schema,
            expected: KEY_LEN,
            actual: data.len(),
        });
    }

    let (prev, last) = data.split_at(HALF_LEN);
    Ok((
        prev.try_into().expect("half width checked"),
        last.try_into().expect("half width checked"),
    ))
}

/// Implements the raw-concatenation key codec for an id type over the given schemas.
macro_rules! impl_id_key_codec {
    ($id:ty, $($schema:ty),+ $(,)?) => {
        $(
            impl KeyCodec<$schema> for $id {
                fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
                    let (prev_block, last_block) = self.raw_parts();
                    Ok(encode_halves(prev_block, last_block))
                }

                fn decode_key(data: &[u8]) -> Result<Self, CodecError> {
                    let (prev_block, last_block) =
                        decode_halves(data, <$schema as Schema>::TREE_NAME.0)?;
                    Ok(Self::from_raw_parts(prev_block, last_block))
                }
            }
        )+
    };
}

impl_id_key_codec!(
    DBBatchId,
    BatchIdToIdxSchema,
    BatchChunksSchema,
    AcctProofReceiptSchema,
);
impl_id_key_codec!(DBChunkId, ChunkIdToIdxSchema);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_id_key_is_raw_concatenation() {
        let id = DBBatchId::from_raw_parts([0x11; 32], [0x22; 32]);
        let encoded = <DBBatchId as KeyCodec<BatchIdToIdxSchema>>::encode_key(&id).expect("encode");

        assert_eq!(encoded, [vec![0x11; 32], vec![0x22; 32]].concat());
        assert_eq!(
            <DBBatchId as KeyCodec<BatchIdToIdxSchema>>::decode_key(&encoded).expect("decode"),
            id
        );
    }

    #[test]
    fn chunk_id_key_is_raw_concatenation() {
        let id = DBChunkId::from_raw_parts([0xAA; 32], [0xBB; 32]);
        let encoded = <DBChunkId as KeyCodec<ChunkIdToIdxSchema>>::encode_key(&id).expect("encode");

        assert_eq!(encoded, [vec![0xAA; 32], vec![0xBB; 32]].concat());
        assert_eq!(
            <DBChunkId as KeyCodec<ChunkIdToIdxSchema>>::decode_key(&encoded).expect("decode"),
            id
        );
    }

    #[test]
    fn key_decode_rejects_wrong_length() {
        assert!(<DBBatchId as KeyCodec<BatchIdToIdxSchema>>::decode_key(&[0u8; 63]).is_err());
        assert!(<DBBatchId as KeyCodec<BatchIdToIdxSchema>>::decode_key(&[0u8; 65]).is_err());
    }
}
