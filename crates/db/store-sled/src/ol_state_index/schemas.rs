//! Schema definitions for the OL state indexing database.

use strata_db_types::ol_state_index::{
    AccountEpochKey, AccountInboxEntry, AccountUpdateEntry, EpochIndexingData,
};
use strata_identifiers::{AccountId, Epoch};

use crate::{
    define_table_with_default_codec, define_table_without_codec, impl_borsh_key_codec,
    impl_cbor_value_codec,
};

define_table_without_codec!(
    /// Maps [`Epoch`] to the epoch's common indexing data.
    (OLEpochIndexingDataSchema) Epoch => EpochIndexingData
);
impl_cbor_value_codec!(OLEpochIndexingDataSchema, EpochIndexingData);

define_table_without_codec!(
    /// Maps [`AccountEpochKey`] to the per-(account, epoch) update entry.
    (OLAccountUpdateEntrySchema) AccountEpochKey => AccountUpdateEntry
);
impl_borsh_key_codec!(OLAccountUpdateEntrySchema, AccountEpochKey);
impl_cbor_value_codec!(OLAccountUpdateEntrySchema, AccountUpdateEntry);

define_table_without_codec!(
    /// Maps [`AccountEpochKey`] to the per-(account, epoch) inbox entry.
    (OLAccountInboxEntrySchema) AccountEpochKey => AccountInboxEntry
);
impl_borsh_key_codec!(OLAccountInboxEntrySchema, AccountEpochKey);
impl_cbor_value_codec!(OLAccountInboxEntrySchema, AccountInboxEntry);

define_table_with_default_codec!(
    /// Maps [`AccountId`] to the epoch in which it was created.
    (OLAccountCreationEpochSchema) AccountId => Epoch
);
