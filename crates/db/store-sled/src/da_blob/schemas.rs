//! Schema definitions for DA blob database tables.

use strata_db_types::types::{DaBlobEntry, DaChunkEntry};
use strata_primitives::buf::Buf32;
use typed_sled::codec::{CodecError, KeyCodec, ValueCodec};

use crate::{
    define_table_with_default_codec, define_table_with_integer_key, define_table_without_codec,
    impl_borsh_value_codec,
};

define_table_with_default_codec!(
    /// Table mapping blob_id to DaBlobEntry.
    (DaBlobSchema) Buf32 => DaBlobEntry
);

define_table_with_integer_key!(
    /// Table mapping chunk index to DaChunkEntry.
    (DaChunkSchema) u64 => DaChunkEntry
);

// For the tag -> wtxid mapping, we need to implement codecs manually
// since [u8; 4] doesn't implement BorshSerialize by default.
define_table_without_codec!(
    /// Table mapping OP_RETURN tag to last chunk wtxid (for cross-blob linking).
    /// Key is 4-byte tag, value is 32-byte wtxid.
    (DaLastChunkWtxidSchema) [u8; 4] => [u8; 32]
);

impl KeyCodec<DaLastChunkWtxidSchema> for [u8; 4] {
    fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
        Ok(self.to_vec())
    }

    fn decode_key(data: &[u8]) -> Result<Self, CodecError> {
        if data.len() != 4 {
            return Err(CodecError::SerializationFailed {
                schema: "DaLastChunkWtxidSchema",
                source: anyhow::anyhow!("expected 4 bytes for tag").into(),
            });
        }
        let mut arr = [0u8; 4];
        arr.copy_from_slice(data);
        Ok(arr)
    }
}

impl ValueCodec<DaLastChunkWtxidSchema> for [u8; 32] {
    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        Ok(self.to_vec())
    }

    fn decode_value(data: &[u8]) -> Result<Self, CodecError> {
        if data.len() != 32 {
            return Err(CodecError::SerializationFailed {
                schema: "DaLastChunkWtxidSchema",
                source: anyhow::anyhow!("expected 32 bytes for wtxid").into(),
            });
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(data);
        Ok(arr)
    }
}
