//! Schema definitions for the OL state indexing database.

use strata_db_types::ol_state_index::{
    AccountEpochKey, AccountUpdateRecord, EpochIndexingData, InboxMessageRecord,
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
    /// Maps [`AccountEpochKey`] to the per-(account, epoch) update records.
    (OLAccountUpdateEntrySchema) AccountEpochKey => Vec<AccountUpdateRecord>
);
impl_borsh_key_codec!(OLAccountUpdateEntrySchema, AccountEpochKey);
impl_cbor_value_codec!(OLAccountUpdateEntrySchema, Vec<AccountUpdateRecord>);

define_table_without_codec!(
    /// Maps [`AccountEpochKey`] to the per-(account, epoch) inbox records.
    (OLAccountInboxEntrySchema) AccountEpochKey => Vec<InboxMessageRecord>
);
impl_borsh_key_codec!(OLAccountInboxEntrySchema, AccountEpochKey);
impl_cbor_value_codec!(OLAccountInboxEntrySchema, Vec<InboxMessageRecord>);

define_table_with_default_codec!(
    /// Maps [`AccountId`] to the epoch in which it was created.
    (OLAccountCreationEpochSchema) AccountId => Epoch
);
