use strata_acct_types::AccountId;
use strata_codec::{Codec, decode_buf_exact, encode_to_vec};
use strata_snark_acct_types::{MessageEntry, MessageEntryProof};
use typed_sled::codec::{CodecError as SledCodecError, KeyCodec};

use crate::{define_table_without_codec, impl_borsh_value_codec};

/// Composite key for message entries: (AccountId, index)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Codec)]
pub struct MessageEntryKey {
    pub account_id: AccountId,
    pub index: u64,
}

define_table_without_codec!(
    /// A table to store message entries by account ID and index
    (MessageEntrySchema) MessageEntryKey => MessageEntry
);

define_table_without_codec!(
    /// A table to store message entry proofs by account ID and index
    (MessageEntryProofSchema) MessageEntryKey => MessageEntryProof
);

// Implement KeyCodec for MessageEntryKey
impl KeyCodec<MessageEntrySchema> for MessageEntryKey {
    fn encode_key(&self) -> Result<Vec<u8>, SledCodecError> {
        encode_to_vec(self).map_err(|e| SledCodecError::SerializationFailed {
            schema: MessageEntrySchema::tree_name(),
            source: format!("Failed to encode MessageEntryKey: {:?}", e).into(),
        })
    }

    fn decode_key(data: &[u8]) -> Result<Self, SledCodecError> {
        decode_buf_exact(data).map_err(|e| SledCodecError::SerializationFailed {
            schema: MessageEntrySchema::tree_name(),
            source: format!("Failed to decode MessageEntryKey: {:?}", e).into(),
        })
    }
}

impl KeyCodec<MessageEntryProofSchema> for MessageEntryKey {
    fn encode_key(&self) -> Result<Vec<u8>, SledCodecError> {
        encode_to_vec(self).map_err(|e| SledCodecError::SerializationFailed {
            schema: MessageEntryProofSchema::tree_name(),
            source: format!("Failed to encode MessageEntryKey: {:?}", e).into(),
        })
    }

    fn decode_key(data: &[u8]) -> Result<Self, SledCodecError> {
        decode_buf_exact(data).map_err(|e| SledCodecError::SerializationFailed {
            schema: MessageEntryProofSchema::tree_name(),
            source: format!("Failed to decode MessageEntryKey: {:?}", e).into(),
        })
    }
}

// Use Borsh value codec (from macros)
// MessageEntry and MessageEntryProof implement BorshSerialize/BorshDeserialize in their own crate
impl_borsh_value_codec!(MessageEntrySchema, MessageEntry);
impl_borsh_value_codec!(MessageEntryProofSchema, MessageEntryProof);
