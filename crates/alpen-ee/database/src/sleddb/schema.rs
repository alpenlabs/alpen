use strata_acct_types::Hash;
use strata_db_store_sled::{
    define_table_with_default_codec, define_table_without_codec, /* impl_bincode_key_codec, */
    impl_borsh_value_codec,
};
use strata_paas::TaskRecordData;
use zkaleido::ProofReceiptWithMetadata;

use crate::serialization_types::{
    DBAccountStateAtEpoch, DBBatchId, DBBatchWithStatus, DBChunkId, DBChunkWithStatus,
    DBExecBlockRecord, DBOLBlockId,
};

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

// Batch storage schemas

define_table_without_codec!(
    /// Batch by sequential idx -> (Batch, Status)
    (BatchByIdxSchema) u64 => DBBatchWithStatus
);
impl_borsh_value_codec!(BatchByIdxSchema, DBBatchWithStatus);

define_table_with_default_codec!(
    /// BatchId -> idx lookup
    (BatchIdToIdxSchema) DBBatchId => u64
);

define_table_without_codec!(
    /// Chunk by sequential idx -> (Chunk, Status)
    (ChunkByIdxSchema) u64 => DBChunkWithStatus
);
impl_borsh_value_codec!(ChunkByIdxSchema, DBChunkWithStatus);

define_table_with_default_codec!(
    /// ChunkId -> idx lookup
    (ChunkIdToIdxSchema) DBChunkId => u64
);

define_table_with_default_codec!(
    /// Batch-Chunk association
    (BatchChunksSchema) DBBatchId => Vec<DBChunkId>
);

// Prover storage schemas.
//
// `ProverTaskSchema` backs `strata_paas::TaskStore` for the EE chunk
// and acct provers; both write under the same tree. Task keys are
// tagged by kind (`b'c'`/`b'a'`) inside `Task::into()` so chunk and
// batch entries don't collide.
//
// The two proof stores are separate trees keyed by domain identifier:
// chunk receipts by task key bytes (matches paas's `ReceiptStore`),
// acct proofs by `DBBatchId`. `AcctProofIdIndexSchema` is a secondary
// index from `ProofId` (= batch's `last_block`) back to the batch, so
// `BatchProver::get_proof(proof_id)` is an O(1) lookup.

define_table_with_default_codec!(
    /// Shared prover task store for chunk + acct provers.
    ///
    /// Keyed by the serialized `ProofSpec::Task` bytes; tag-prefixed on
    /// the caller side (`ChunkTask` / `BatchTask`).
    (ProverTaskSchema) Vec<u8> => TaskRecordData
);

define_table_with_default_codec!(
    /// Chunk proof receipts, keyed by chunk task bytes.
    ///
    /// The acct `fetch_input` reads these to assemble chunk inputs. Key
    /// shape matches paas's `ReceiptStore`.
    (ChunkProofReceiptSchema) Vec<u8> => ProofReceiptWithMetadata
);

define_table_with_default_codec!(
    /// Acct (outer/update) proof receipts keyed by [`BatchId`].
    (AcctProofReceiptSchema) DBBatchId => ProofReceiptWithMetadata
);

define_table_with_default_codec!(
    /// Secondary index: `ProofId` → `BatchId`, so `BatchProver::get_proof`
    /// can resolve the receipt without scanning.
    (AcctProofIdIndexSchema) Hash => DBBatchId
);
