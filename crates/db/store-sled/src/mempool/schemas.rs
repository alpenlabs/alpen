use strata_codec::{decode_buf_exact, encode_to_vec};
use strata_identifiers::OLTxId;
use strata_ol_chain_types_new::OLTransaction;
use typed_sled::codec::{CodecError as SledCodecError, KeyCodec, ValueCodec};

use crate::define_table_without_codec;

define_table_without_codec!(
    /// A table to store mempool transactions (OLTransaction type)
    (MempoolTxSchema) OLTxId => OLTransaction
);

// Implement KeyCodec for OLTxId
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

// Implement ValueCodec for OLTransaction
impl ValueCodec<MempoolTxSchema> for OLTransaction {
    fn encode_value(&self) -> Result<Vec<u8>, SledCodecError> {
        encode_to_vec(self).map_err(|e| SledCodecError::SerializationFailed {
            schema: MempoolTxSchema::tree_name(),
            source: format!("Failed to encode OLTransaction: {:?}", e).into(),
        })
    }

    fn decode_value(data: &[u8]) -> Result<Self, SledCodecError> {
        decode_buf_exact(data).map_err(|e| SledCodecError::SerializationFailed {
            schema: MempoolTxSchema::tree_name(),
            source: format!("Failed to decode OLTransaction: {:?}", e).into(),
        })
    }
}
