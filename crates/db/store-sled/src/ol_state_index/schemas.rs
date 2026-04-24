//! Schema definitions for the OL state indexing database.

use strata_db_types::ol_state_index::{
    AccountEpochRecord, BlockIndexingRecord, CommonEpochRecord,
};
use strata_identifiers::{AccountId, Epoch, OLBlockCommitment};

use crate::{
    define_table_with_default_codec, define_table_without_codec, impl_borsh_key_codec,
    impl_cbor_value_codec,
};

define_table_without_codec!(
    /// Maps [`Epoch`] to the epoch's common indexing record.
    (OLCommonEpochSchema) Epoch => CommonEpochRecord
);
impl_cbor_value_codec!(OLCommonEpochSchema, CommonEpochRecord);

define_table_without_codec!(
    /// Maps [`(AccountId, Epoch)`] to the per-account per-epoch record.
    (OLAccountEpochSchema) (AccountId, Epoch) => AccountEpochRecord
);
impl_borsh_key_codec!(OLAccountEpochSchema, (AccountId, Epoch));
impl_cbor_value_codec!(OLAccountEpochSchema, AccountEpochRecord);

define_table_with_default_codec!(
    /// Maps [`AccountId`] to the epoch in which it was created.
    (OLAccountCreationEpochSchema) AccountId => Epoch
);

// Block indexing: keyed by (epoch, block commitment) with big-endian bincode
// so we can prefix-scan all blocks in an epoch.
define_table_without_codec!(
    /// Per-block indexing record keyed by (epoch, block commitment).
    (OLBlockIndexingSchema) (Epoch, OLBlockCommitment) => BlockIndexingRecord
);

impl ::typed_sled::codec::KeyCodec<OLBlockIndexingSchema> for (Epoch, OLBlockCommitment) {
    fn encode_key(&self) -> Result<Vec<u8>, ::typed_sled::codec::CodecError> {
        use ::bincode::Options as _;
        let opts = ::bincode::options()
            .with_fixint_encoding()
            .with_big_endian();
        opts.serialize(self).map_err(|err| {
            ::typed_sled::codec::CodecError::SerializationFailed {
                schema: OLBlockIndexingSchema::tree_name(),
                source: err,
            }
        })
    }

    fn decode_key(data: &[u8]) -> Result<Self, ::typed_sled::codec::CodecError> {
        use ::bincode::Options as _;
        let opts = ::bincode::options()
            .with_fixint_encoding()
            .with_big_endian();
        opts.deserialize(data).map_err(|err| {
            ::typed_sled::codec::CodecError::SerializationFailed {
                schema: OLBlockIndexingSchema::tree_name(),
                source: err,
            }
        })
    }
}

impl_cbor_value_codec!(OLBlockIndexingSchema, BlockIndexingRecord);
