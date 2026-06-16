use strata_db_types::{LeafPos, NodePos, RawMmrId};
use strata_primitives::buf::Buf32;
use typed_sled::codec::{CodecError, KeyCodec};

use crate::{define_table_with_seek_key_codec, define_table_without_codec, impl_borsh_value_codec};

define_table_without_codec!(
    /// MMR index node storage: (mmr_id, node_pos) -> hash
    (MmrIndexNodeSchema) (RawMmrId, NodePos) => Buf32
);

define_table_without_codec!(
    /// MMR index preimage storage: (mmr_id, leaf_pos) -> preimage bytes
    (MmrIndexPreimageSchema) (RawMmrId, LeafPos) => Vec<u8>
);

define_table_with_seek_key_codec!(
    /// MMR index leaf count storage: mmr_id -> leaf count
    (MmrIndexLeafCountSchema) RawMmrId => u64
);

impl_borsh_value_codec!(MmrIndexNodeSchema, Buf32);
impl_borsh_value_codec!(MmrIndexPreimageSchema, Vec<u8>);

// The position types live in `strata-merkle-node-store` and carry no `serde`
// impls, so the generic bincode seek-key codec cannot be used for the scoped
// `(mmr_id, pos)` keys. Encode them by hand instead, using the position's own
// `to_key`/`index` byte layout.
//
// The byte layout is identical to the previous bincode (fixint, big-endian)
// encoding — `mmr_id_len(u64 BE) || mmr_id_bytes || pos_bytes`, where
// `pos_bytes` is `height || index_be` for a node and `index_be` for a leaf —
// so existing databases need no migration. The regression tests below pin this.

/// Frames a scoped key as `mmr_id_len(u64 BE) || mmr_id_bytes || tail`.
fn encode_scoped_key(mmr_id: &RawMmrId, tail: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + mmr_id.len() + tail.len());
    out.extend_from_slice(&(mmr_id.len() as u64).to_be_bytes());
    out.extend_from_slice(mmr_id);
    out.extend_from_slice(tail);
    out
}

/// Splits a scoped key into its `mmr_id` and the fixed-length position tail.
fn split_scoped_key<'a>(
    data: &'a [u8],
    tail_len: usize,
    schema: &'static str,
) -> Result<(RawMmrId, &'a [u8]), CodecError> {
    let len_prefix = 8;
    if data.len() < len_prefix {
        return Err(CodecError::InvalidKeyLength {
            schema,
            expected: len_prefix,
            actual: data.len(),
        });
    }

    let id_len = u64::from_be_bytes(data[..len_prefix].try_into().expect("8-byte length prefix"));
    let id_len = usize::try_from(id_len).map_err(|_| CodecError::InvalidKeyLength {
        schema,
        expected: usize::MAX,
        actual: data.len(),
    })?;

    let expected = len_prefix + id_len + tail_len;
    if data.len() != expected {
        return Err(CodecError::InvalidKeyLength {
            schema,
            expected,
            actual: data.len(),
        });
    }

    let mmr_id = data[len_prefix..len_prefix + id_len].to_vec();
    let tail = &data[len_prefix + id_len..];
    Ok((mmr_id, tail))
}

impl KeyCodec<MmrIndexNodeSchema> for (RawMmrId, NodePos) {
    fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
        Ok(encode_scoped_key(&self.0, &self.1.to_key()))
    }

    fn decode_key(data: &[u8]) -> Result<Self, CodecError> {
        let (mmr_id, tail) = split_scoped_key(data, 9, MmrIndexNodeSchema::tree_name())?;
        let height = tail[0];
        let index = u64::from_be_bytes(tail[1..9].try_into().expect("8-byte index"));
        Ok((mmr_id, NodePos::new(height, index)))
    }
}

impl KeyCodec<MmrIndexPreimageSchema> for (RawMmrId, LeafPos) {
    fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
        Ok(encode_scoped_key(&self.0, &self.1.index().to_be_bytes()))
    }

    fn decode_key(data: &[u8]) -> Result<Self, CodecError> {
        let (mmr_id, tail) = split_scoped_key(data, 8, MmrIndexPreimageSchema::tree_name())?;
        let index = u64::from_be_bytes(tail[..8].try_into().expect("8-byte index"));
        Ok((mmr_id, LeafPos::new(index)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The expected byte vectors below pin the exact on-disk key layout, which
    // matches the prior bincode (fixint, big-endian) encoding of the old
    // serde-backed keys: `mmr_id_len(u64 BE) || mmr_id_bytes || pos_bytes`.
    // Any drift here means existing databases would silently fail to read.

    #[test]
    fn node_key_matches_on_disk_layout() {
        let mmr_id: RawMmrId = vec![0xAA, 0xBB, 0xCC];
        let pos = NodePos::new(2, 0x0102_0304_0506_0708);

        let encoded = (mmr_id.clone(), pos).encode_key().expect("encode node key");
        #[rustfmt::skip]
        let expected = vec![
            0, 0, 0, 0, 0, 0, 0, 3, // mmr_id length (u64 BE)
            0xAA, 0xBB, 0xCC,       // mmr_id bytes
            0x02,                   // node height
            1, 2, 3, 4, 5, 6, 7, 8, // node index (u64 BE)
        ];
        assert_eq!(
            encoded, expected,
            "node key layout drifted from on-disk format"
        );

        let (decoded_id, decoded_pos) =
            <(RawMmrId, NodePos)>::decode_key(&encoded).expect("decode node key");
        assert_eq!(decoded_id, mmr_id);
        assert_eq!(decoded_pos, pos);
    }

    #[test]
    fn leaf_key_matches_on_disk_layout() {
        let mmr_id: RawMmrId = vec![0x01, 0x02];
        let leaf = LeafPos::new(0x0807_0605_0403_0201);

        let encoded = (mmr_id.clone(), leaf)
            .encode_key()
            .expect("encode leaf key");
        #[rustfmt::skip]
        let expected = vec![
            0, 0, 0, 0, 0, 0, 0, 2, // mmr_id length (u64 BE)
            1, 2,                   // mmr_id bytes
            8, 7, 6, 5, 4, 3, 2, 1, // leaf index (u64 BE)
        ];
        assert_eq!(
            encoded, expected,
            "leaf key layout drifted from on-disk format"
        );

        let (decoded_id, decoded_leaf) =
            <(RawMmrId, LeafPos)>::decode_key(&encoded).expect("decode leaf key");
        assert_eq!(decoded_id, mmr_id);
        assert_eq!(decoded_leaf, leaf);
    }

    #[test]
    fn empty_mmr_id_round_trips() {
        let mmr_id: RawMmrId = vec![];
        let pos = NodePos::new(0, 0);
        let encoded = (mmr_id.clone(), pos).encode_key().expect("encode");
        let (decoded_id, decoded_pos) =
            <(RawMmrId, NodePos)>::decode_key(&encoded).expect("decode");
        assert_eq!(decoded_id, mmr_id);
        assert_eq!(decoded_pos, pos);
    }
}
