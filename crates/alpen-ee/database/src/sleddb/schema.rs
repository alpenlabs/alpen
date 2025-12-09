use strata_acct_types::Hash;
use strata_db_store_sled::{
    define_table_with_default_codec, define_table_without_codec, /* impl_bincode_key_codec, */
    impl_borsh_value_codec,
};

use crate::serialization_types::{DBAccountStateAtEpoch, DBExecBlockRecord, DBOLBlockId};

define_table_without_codec!(
    /// store canonical final OL block id at OL epoch
    (OLBlockAtEpochSchema) u32 => DBOLBlockId
);
// impl_bincode_key_codec!(OLBlockAtEpochSchema, u32);
impl_borsh_value_codec!(OLBlockAtEpochSchema, DBOLBlockId);

define_table_with_default_codec!(
    /// EeAccountState at specific OL Block
    (AccountStateAtOLEpochSchema) DBOLBlockId => DBAccountStateAtEpoch
);

define_table_with_default_codec!(
    /// ExecBlock by blockhash
    (ExecBlockSchema) Hash => DBExecBlockRecord
);

define_table_without_codec!(
    /// All ExecBlocks by height
    (ExecBlocksAtHeightSchema) u64 => Vec<Hash>
);
impl_borsh_value_codec!(ExecBlocksAtHeightSchema, Vec<Hash>);

define_table_without_codec!(
    /// Canonical finalized chain by height
    (ExecBlockFinalizedSchema) u64 => Hash
);
impl_borsh_value_codec!(ExecBlockFinalizedSchema, Hash);

define_table_with_default_codec!(
    /// ExecBlock payloads
    (ExecBlockPayloadSchema) Hash => Vec<u8>
);
