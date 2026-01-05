use ssz_derive::{Decode, Encode};
use strata_identifiers::OLTxId;
use typed_sled::codec::{CodecError, KeyCodec, ValueCodec};

use crate::define_table_without_codec;

/// Wrapper type for storing transaction bytes and ordering metadata.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub(crate) struct MempoolTxEntry {
    /// Raw transaction bytes.
    pub(crate) tx_bytes: Vec<u8>,
    /// Timestamp (microseconds since UNIX epoch) for FIFO ordering.
    ///
    /// Persists across restarts.
    pub(crate) timestamp_micros: u64,
}

impl MempoolTxEntry {
    pub(crate) fn new(tx_bytes: Vec<u8>, timestamp_micros: u64) -> Self {
        Self {
            tx_bytes,
            timestamp_micros,
        }
    }

    pub(crate) fn into_tuple(self) -> (Vec<u8>, u64) {
        (self.tx_bytes, self.timestamp_micros)
    }
}

define_table_without_codec!(
    /// A table to store mempool transactions.
    /// Maps [`OLTxId`] => [`MempoolTxEntry`]
    (MempoolTxSchema) OLTxId => MempoolTxEntry
);

// Use SSZ encoding for the key (OLTxId)
impl KeyCodec<MempoolTxSchema> for OLTxId {
    fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
        Ok(ssz::Encode::as_ssz_bytes(self))
    }

    fn decode_key(data: &[u8]) -> Result<Self, CodecError> {
        ssz::Decode::from_ssz_bytes(data).map_err(|err| CodecError::SerializationFailed {
            schema: MempoolTxSchema::tree_name(),
            source: anyhow::anyhow!("SSZ decode error for key: {:?}", err).into(),
        })
    }
}

// Use SSZ encoding for the value (MempoolTxEntry)
impl ValueCodec<MempoolTxSchema> for MempoolTxEntry {
    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        Ok(ssz::Encode::as_ssz_bytes(self))
    }

    fn decode_value(data: &[u8]) -> Result<Self, CodecError> {
        ssz::Decode::from_ssz_bytes(data).map_err(|err| CodecError::SerializationFailed {
            schema: MempoolTxSchema::tree_name(),
            source: anyhow::anyhow!("SSZ decode error: {:?}", err).into(),
        })
    }
}
