use strata_db_store_rocksdb::{define_table_with_default_codec, define_table_with_seek_key_codec};

use crate::db::serialization_types::{DBAccountStateAtSlot, DBOLBlockId};

define_table_with_seek_key_codec!(
    /// store canonical OL block id at OL slot
    (OlBlockAtSlotSchema) u64 => DBOLBlockId
);

define_table_with_default_codec!(
    /// EeAccountState at specific OL Block
    (AccountStateAtOlBlockSchema) DBOLBlockId => DBAccountStateAtSlot
);
