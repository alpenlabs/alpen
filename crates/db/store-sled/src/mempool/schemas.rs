use strata_codec::{decode_buf_exact, encode_to_vec};
use strata_identifiers::OLTxId;
use typed_sled::codec::{CodecError as SledCodecError, KeyCodec, ValueCodec};

use crate::define_table_without_codec;

define_table_without_codec!(
    /// A table to store mempool transactions (raw blobs only, no metadata)
    (MempoolTxSchema) OLTxId => Vec<u8>
);

// Implement KeyCodec for OLTxId using Codec trait
impl KeyCodec<MempoolTxSchema> for OLTxId {
    fn encode_key(&self) -> Result<Vec<u8>, SledCodecError> {
        encode_to_vec(self).map_err(|e| SledCodecError::SerializationFailed {
            schema: MempoolTxSchema::tree_name(),
            source: format!("Failed to encode OLTxId: {:?}", e).into(),
        })
    }

    fn decode_key(data: &[u8]) -> Result<Self, SledCodecError> {
        decode_buf_exact(data).map_err(|e| SledCodecError::SerializationFailed {
            schema: MempoolTxSchema::tree_name(),
            source: format!("Failed to decode OLTxId: {:?}", e).into(),
        })
    }
}

// Implement ValueCodec for Vec<u8> (raw transaction blob)
// We store the blob directly without any encoding/decoding
impl ValueCodec<MempoolTxSchema> for Vec<u8> {
    fn encode_value(&self) -> Result<Vec<u8>, SledCodecError> {
        // Just return the blob as-is
        Ok(self.clone())
    }

    fn decode_value(data: &[u8]) -> Result<Self, SledCodecError> {
        // Just return the blob as-is
        Ok(data.to_vec())
    }
}
