use revm_primitives::alloy_primitives::B256;
use strata_db_store_sled::{define_table_without_codec, impl_bincode_key_codec};
use typed_sled::codec::{CodecError, ValueCodec};

// First define the table structures without codecs
define_table_without_codec!(
    /// store of block witness data. Data stored as serialized bytes for directly serving in rpc
    (BlockWitnessSchema) B256 => Vec<u8>
);

define_table_without_codec!(
    /// store of block state diff data. Data stored as serialized bytes for directly serving in rpc
    (BlockStateChangesSchema) B256 => Vec<u8>
);

define_table_without_codec!(
    /// block number => hash mapping for easier testing.
    (BlockHashByNumber) u64 => Vec<u8>
);

define_table_without_codec!(
    /// Set of contract code hashes already published to DA.
    /// Key is the code hash; value is unused (presence-only).
    (PublishedCodeHashSchema) B256 => Vec<u8>
);

// B256 key codec — big-endian serialization for lexicographic ordering
impl_bincode_key_codec!(BlockWitnessSchema, B256);
impl_bincode_key_codec!(BlockStateChangesSchema, B256);
impl_bincode_key_codec!(PublishedCodeHashSchema, B256);

// Vec<u8> value codec - stored as raw bytes
impl ValueCodec<BlockWitnessSchema> for Vec<u8> {
    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        Ok(self.clone())
    }

    fn decode_value(data: &[u8]) -> Result<Self, CodecError> {
        Ok(data.to_vec())
    }
}

impl ValueCodec<BlockStateChangesSchema> for Vec<u8> {
    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        Ok(self.clone())
    }

    fn decode_value(data: &[u8]) -> Result<Self, CodecError> {
        Ok(data.to_vec())
    }
}

impl ValueCodec<BlockHashByNumber> for Vec<u8> {
    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        Ok(self.clone())
    }

    fn decode_value(data: &[u8]) -> Result<Self, CodecError> {
        Ok(data.to_vec())
    }
}

impl ValueCodec<PublishedCodeHashSchema> for Vec<u8> {
    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        Ok(self.clone())
    }

    fn decode_value(data: &[u8]) -> Result<Self, CodecError> {
        Ok(data.to_vec())
    }
}
