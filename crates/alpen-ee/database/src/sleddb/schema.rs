use strata_acct_types::Hash;
use strata_db_store_sled::{
    define_table_with_default_codec, define_table_with_integer_key, define_table_without_codec,
    impl_bytes_value_codec, impl_integer_value_codec, impl_rkyv_key_codec,
};

use crate::serialization_types::{
    DBAccountStateAtEpoch, DBBatchId, DBBatchWithStatus, DBChunkId, DBChunkWithStatus,
    DBExecBlockRecord, DBOLBlockId,
};

define_table_with_integer_key!(
    /// store canonical final OL block id at OL epoch
    (OLBlockAtEpochSchema) u32 => DBOLBlockId
);

define_table_with_default_codec!(
    /// EeAccountState at specific OL Block
    (AccountStateAtOLEpochSchema) DBOLBlockId => DBAccountStateAtEpoch
);

define_table_with_default_codec!(
    /// ExecBlock by blockhash
    (ExecBlockSchema) Hash => DBExecBlockRecord
);

define_table_with_integer_key!(
    /// All ExecBlocks by height
    (ExecBlocksAtHeightSchema) u64 => Vec<Hash>
);

define_table_with_integer_key!(
    /// Canonical finalized chain by height
    (ExecBlockFinalizedSchema) u64 => Hash
);

define_table_without_codec!(
    /// ExecBlock payloads
    (ExecBlockPayloadSchema) Hash => Vec<u8>
);
impl_rkyv_key_codec!(ExecBlockPayloadSchema, Hash);
impl_bytes_value_codec!(ExecBlockPayloadSchema);

// Batch storage schemas

define_table_with_integer_key!(
    /// Batch by sequential idx -> (Batch, Status)
    (BatchByIdxSchema) u64 => DBBatchWithStatus
);

define_table_without_codec!(
    /// BatchId -> idx lookup
    (BatchIdToIdxSchema) DBBatchId => u64
);
impl_rkyv_key_codec!(BatchIdToIdxSchema, DBBatchId);
impl_integer_value_codec!(BatchIdToIdxSchema, u64);

define_table_with_integer_key!(
    /// Chunk by sequential idx -> (Chunk, Status)
    (ChunkByIdxSchema) u64 => DBChunkWithStatus
);

define_table_without_codec!(
    /// ChunkId -> idx lookup
    (ChunkIdToIdxSchema) DBChunkId => u64
);
impl_rkyv_key_codec!(ChunkIdToIdxSchema, DBChunkId);
impl_integer_value_codec!(ChunkIdToIdxSchema, u64);

define_table_with_default_codec!(
    /// Batch-Chunk association
    (BatchChunksSchema) DBBatchId => Vec<DBChunkId>
);
