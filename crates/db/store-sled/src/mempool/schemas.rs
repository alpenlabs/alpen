use ssz_derive::{Decode, Encode};
use strata_identifiers::OLTxId;
use typed_sled::codec::{CodecError, KeyCodec, ValueCodec};

use crate::define_table_without_codec;

/// Wrapper type for storing transaction bytes and ordering metadata.
///
/// Stored using SSZ encoding. This avoids Borsh serialization and stores the
/// SSZ-encoded transaction bytes directly (the transaction bytes themselves are
/// already SSZ-encoded `OLMempoolTransaction`).
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub(crate) struct MempoolTxEntry {
    /// Raw transaction bytes (SSZ-encoded `OLMempoolTransaction`).
    pub(crate) tx_bytes: Vec<u8>,
    /// Slot when this transaction was first seen/added to the mempool.
    pub(crate) first_seen_slot: u64,
    /// Monotonic counter for FIFO ordering within same slot.
    pub(crate) insertion_id: u64,
}

impl MempoolTxEntry {
    pub(crate) fn new(tx_bytes: Vec<u8>, first_seen_slot: u64, insertion_id: u64) -> Self {
        Self {
            tx_bytes,
            first_seen_slot,
            insertion_id,
        }
    }

    pub(crate) fn into_tuple(self) -> (Vec<u8>, u64, u64) {
        (self.tx_bytes, self.first_seen_slot, self.insertion_id)
    }
}

define_table_without_codec!(
    /// A table to store mempool transactions.
    /// Maps OLTxId => (tx_bytes, first_seen_slot)
    /// Key: SSZ-encoded OLTxId
    /// Value: SSZ-encoded MempoolTxEntry
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
