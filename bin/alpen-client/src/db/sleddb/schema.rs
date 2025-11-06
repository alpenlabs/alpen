use strata_db_store_sled::{
    define_table_with_default_codec, define_table_without_codec, /* impl_bincode_key_codec, */
    impl_borsh_value_codec,
};

use crate::db::serialization_types::{DBAccountStateAtSlot, DBOLBlockId};

define_table_without_codec!(
    /// store canonical OL block id at OL slot
    (OlBlockAtSlotSchema) u64 => DBOLBlockId
);
// impl_bincode_key_codec!(OlBlockAtSlotSchema, u64);
impl_borsh_value_codec!(OlBlockAtSlotSchema, DBOLBlockId);

define_table_with_default_codec!(
    /// EeAccountState at specific OL Block
    (AccountStateAtOlBlockSchema) DBOLBlockId => DBAccountStateAtSlot
);
