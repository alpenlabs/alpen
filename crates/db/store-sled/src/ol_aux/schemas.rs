//! Schema definitions for OL auxiliary data storage.

use bincode::Options;
use ssz::{Decode, Encode};
use strata_identifiers::AccountId;
use strata_snark_acct_types::MessageEntry;
use typed_sled::codec::{CodecError, KeyCodec, ValueCodec};

use crate::define_table_without_codec;

/// Key type for inbox messages: (AccountId, message_index).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InboxMessageKey {
    pub account_id: AccountId,
    pub index: u64,
}

impl InboxMessageKey {
    pub fn new(account_id: AccountId, index: u64) -> Self {
        Self { account_id, index }
    }
}

define_table_without_codec!(
    /// A table to store inbox message entries.
    /// Maps (AccountId, index) to MessageEntry.
    (InboxMessageSchema) InboxMessageKey => MessageEntry
);

// Use big-endian encoding for the key to ensure lexicographic ordering by account then index.
impl KeyCodec<InboxMessageSchema> for InboxMessageKey {
    fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
        let bincode_options = bincode::options().with_fixint_encoding().with_big_endian();

        bincode_options
            .serialize(&(*self.account_id.inner(), self.index))
            .map_err(|err| CodecError::SerializationFailed {
                schema: InboxMessageSchema::tree_name(),
                source: err.into(),
            })
    }

    fn decode_key(data: &[u8]) -> Result<Self, CodecError> {
        let bincode_options = bincode::options().with_fixint_encoding().with_big_endian();

        let (account_bytes, index): ([u8; 32], u64) = bincode_options
            .deserialize_from(&mut &data[..])
            .map_err(|err| CodecError::SerializationFailed {
                schema: InboxMessageSchema::tree_name(),
                source: err.into(),
            })?;

        Ok(Self {
            account_id: AccountId::from(account_bytes),
            index,
        })
    }
}

// MessageEntry is SSZ-generated, so use SSZ serialization.
impl ValueCodec<InboxMessageSchema> for MessageEntry {
    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        Ok(self.as_ssz_bytes())
    }

    fn decode_value(data: &[u8]) -> Result<Self, CodecError> {
        Self::from_ssz_bytes(data).map_err(|err| CodecError::SerializationFailed {
            schema: InboxMessageSchema::tree_name(),
            source: format!("SSZ decode error: {err:?}").into(),
        })
    }
}
