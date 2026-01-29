use revm_primitives::alloy_primitives::B256;
use strata_db_store_sled::define_table_without_codec;
use typed_sled::codec::{CodecError, KeyCodec, ValueCodec};

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

// Custom codec for B256 key using big-endian serialization for ordering
impl KeyCodec<BlockWitnessSchema> for B256 {
    fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
        use bincode::Options;

        let bincode_options = bincode::options().with_fixint_encoding().with_big_endian();

        bincode_options
            .serialize(self)
            .map_err(|err| CodecError::SerializationFailed {
                schema: "BlockWitnessSchema",
                source: err.into(),
            })
    }

    fn decode_key(data: &[u8]) -> Result<Self, CodecError> {
        use bincode::Options;

        let bincode_options = bincode::options().with_fixint_encoding().with_big_endian();

        bincode_options
            .deserialize_from(&mut &data[..])
            .map_err(|err| CodecError::SerializationFailed {
                schema: "BlockWitnessSchema",
                source: err.into(),
            })
    }
}

impl KeyCodec<BlockStateChangesSchema> for B256 {
    fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
        use bincode::Options;

        let bincode_options = bincode::options().with_fixint_encoding().with_big_endian();

        bincode_options
            .serialize(self)
            .map_err(|err| CodecError::SerializationFailed {
                schema: "BlockStateChangesSchema",
                source: err.into(),
            })
    }

    fn decode_key(data: &[u8]) -> Result<Self, CodecError> {
        use bincode::Options;

        let bincode_options = bincode::options().with_fixint_encoding().with_big_endian();

        bincode_options
            .deserialize_from(&mut &data[..])
            .map_err(|err| CodecError::SerializationFailed {
                schema: "BlockStateChangesSchema",
                source: err.into(),
            })
    }
}

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
