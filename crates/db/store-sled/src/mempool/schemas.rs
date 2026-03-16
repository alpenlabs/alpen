use sled::IVec;
use ssz_derive::{Decode, Encode};
use strata_identifiers::OLTxId;
use thiserror::Error;
use typed_sled::codec::{CodecError, KeyCodec, ValueCodec};

use crate::define_table_without_codec;

/// Errors raised while decoding mempool SSZ payloads from sled.
#[derive(Debug, Error)]
enum MempoolDecodeError {
    /// Decoding the SSZ-encoded [`OLTxId`] key failed.
    #[error("failed to decode mempool transaction key from SSZ")]
    Key(#[source] ssz::DecodeError),

    /// Decoding the SSZ-encoded [`MempoolTxEntry`] value failed.
    #[error("failed to decode mempool transaction entry from SSZ")]
    Value(#[source] ssz::DecodeError),
}

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
        ssz::Decode::from_ssz_bytes(data).map_err(|err| CodecError::DeserializationFailed {
            schema: MempoolTxSchema::tree_name(),
            source: Box::new(MempoolDecodeError::Key(err)),
        })
    }
}

// Use SSZ encoding for the value (MempoolTxEntry)
impl ValueCodec<MempoolTxSchema> for MempoolTxEntry {
    type Decoded = Self;

    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        Ok(ssz::Encode::as_ssz_bytes(self))
    }

    fn decode_value(data: IVec) -> Result<Self::Decoded, CodecError> {
        ssz::Decode::from_ssz_bytes(data.as_ref()).map_err(|err| {
            CodecError::DeserializationFailed {
                schema: MempoolTxSchema::tree_name(),
                source: Box::new(MempoolDecodeError::Value(err)),
            }
        })
    }
}
