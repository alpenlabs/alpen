//! Database schemas for OL state storage.

use borsh::{BorshDeserialize, to_vec};
use ssz::{Decode, Encode};
use strata_acct_types::AccountId;
use strata_codec::{decode_buf_exact, encode_to_vec};
use strata_db_types::{OLFinalizedState, OLWriteBatch};
use strata_merkle::CompactMmr64B32;
use strata_primitives::{buf::Buf32, l1::L1Height, l2::OLBlockId};
use strata_snark_acct_types::MessageEntry;
use typed_sled::{CodecError, KeyCodec, ValueCodec};

use crate::{define_table_with_integer_key, define_table_without_codec, impl_borsh_value_codec};

// Custom serialization for WriteBatch using strata-codec
define_table_without_codec!(
    /// Table to store write batches for each block slot.
    ///
    /// Key: OLBlockId, Value: OLWriteBatch
    (SlotWriteBatchSchema) OLBlockId => OLWriteBatch
);

impl KeyCodec<SlotWriteBatchSchema> for OLBlockId {
    fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
        to_vec(self).map_err(Into::into)
    }

    fn decode_key(data: &[u8]) -> Result<Self, CodecError> {
        BorshDeserialize::deserialize_reader(&mut &data[..]).map_err(Into::into)
    }
}

impl ValueCodec<SlotWriteBatchSchema> for OLWriteBatch {
    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        encode_to_vec(self).map_err(|e| CodecError::SerializationFailed {
            schema: "SlotWriteBatchSchema",
            source: e.into(),
        })
    }

    fn decode_value(data: &[u8]) -> Result<Self, CodecError> {
        decode_buf_exact(data).map_err(|e| CodecError::SerializationFailed {
            schema: "SlotWriteBatchSchema",
            source: e.into(),
        })
    }
}

define_table_without_codec!(
    /// Table to store the finalized state.
    ///
    /// Uses a unit key since there's only one finalized state.
    /// Key: (), Value: OLFinalizedState
    (FinalizedStateSchema) () => OLFinalizedState
);

impl KeyCodec<FinalizedStateSchema> for () {
    fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
        Ok(vec![0])
    }

    fn decode_key(_data: &[u8]) -> Result<Self, CodecError> {
        Ok(())
    }
}

impl ValueCodec<FinalizedStateSchema> for OLFinalizedState {
    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        // OLState implements Codec, so we can use strata_codec directly
        encode_to_vec(self).map_err(|e| CodecError::SerializationFailed {
            schema: "FinalizedStateSchema",
            source: e.into(),
        })
    }

    fn decode_value(data: &[u8]) -> Result<Self, CodecError> {
        // OLState implements Codec, so we can use strata_codec directly
        decode_buf_exact(data).map_err(|e| CodecError::SerializationFailed {
            schema: "FinalizedStateSchema",
            source: e.into(),
        })
    }
}

define_table_with_integer_key!(
    /// Table to store manifest entries by L1 height.
    ///
    /// This is an append-only log of manifest hashes keyed by L1 height.
    /// Key: L1Height (u32), Value: Vec<Buf32> (list of manifest hashes at that height)
    (ManifestEntrySchema) L1Height => Vec<Buf32>
);

define_table_without_codec!(
    /// Table to store the manifest MMR in compact form.
    ///
    /// Uses a unit key since there's only one MMR instance.
    /// Key: (), Value: CompactMmr64B32 (SSZ-serialized)
    (ManifestMmrSchema) () => CompactMmr64B32
);

impl KeyCodec<ManifestMmrSchema> for () {
    fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
        Ok(vec![0])
    }

    fn decode_key(_data: &[u8]) -> Result<Self, CodecError> {
        Ok(())
    }
}

impl ValueCodec<ManifestMmrSchema> for CompactMmr64B32 {
    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        Ok(self.as_ssz_bytes())
    }

    fn decode_value(data: &[u8]) -> Result<Self, CodecError> {
        CompactMmr64B32::from_ssz_bytes(data).map_err(|e| CodecError::SerializationFailed {
            schema: "ManifestMmrSchema",
            source: anyhow::anyhow!("SSZ decode failed: {:?}", e).into(),
        })
    }
}

/// Composite key for inbox messages: (AccountId, message index)
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InboxMessageKey {
    acct_id: AccountId,
    msg_idx: u64,
}

impl InboxMessageKey {
    pub(crate) fn new(acct_id: AccountId, msg_idx: u64) -> Self {
        Self { acct_id, msg_idx }
    }
}

define_table_without_codec!(
    /// Table to store inbox messages for snark accounts.
    ///
    /// Key: InboxMessageKey (AccountId, message index), Value: MessageEntry
    (InboxMessageSchema) InboxMessageKey => MessageEntry
);

// Custom codec implementation for InboxMessageKey since AccountId uses Codec not Borsh
impl KeyCodec<InboxMessageSchema> for InboxMessageKey {
    fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
        // Encode account ID
        let acct_id_bytes =
            encode_to_vec(&self.acct_id).map_err(|e| CodecError::SerializationFailed {
                schema: "InboxMessageSchema",
                source: e.into(),
            })?;

        // Encode message index
        let msg_idx_bytes = strata_codec::encode_to_vec(&self.msg_idx).map_err(|e| {
            typed_sled::codec::CodecError::SerializationFailed {
                schema: "InboxMessageSchema",
                source: e.into(),
            }
        })?;

        // Concatenate both
        let mut buf = Vec::with_capacity(acct_id_bytes.len() + msg_idx_bytes.len());
        buf.extend_from_slice(&acct_id_bytes);
        buf.extend_from_slice(&msg_idx_bytes);
        Ok(buf)
    }

    fn decode_key(data: &[u8]) -> Result<Self, CodecError> {
        // AccountId is 32 bytes
        if data.len() < 32 {
            return Err(CodecError::SerializationFailed {
                schema: "InboxMessageSchema",
                source: anyhow::anyhow!("Insufficient data for InboxMessageKey").into(),
            });
        }

        // Decode AccountId from first 32 bytes
        let acct_id =
            decode_buf_exact(&data[..32]).map_err(|e| CodecError::SerializationFailed {
                schema: "InboxMessageSchema",
                source: e.into(),
            })?;

        // Decode msg_idx from remaining bytes
        let msg_idx =
            decode_buf_exact(&data[32..]).map_err(|e| CodecError::SerializationFailed {
                schema: "InboxMessageSchema",
                source: e.into(),
            })?;

        Ok(InboxMessageKey::new(acct_id, msg_idx))
    }
}

// MessageEntry is SSZ-based, so use SSZ encoding
impl ValueCodec<InboxMessageSchema> for MessageEntry {
    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        Ok(self.as_ssz_bytes())
    }

    fn decode_value(data: &[u8]) -> Result<Self, CodecError> {
        MessageEntry::from_ssz_bytes(data).map_err(|e| CodecError::SerializationFailed {
            schema: "InboxMessageSchema",
            source: anyhow::anyhow!("SSZ decode failed: {:?}", e).into(),
        })
    }
}
