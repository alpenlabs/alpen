use strata_checkpoint_types::EpochSummary;
use strata_db_types::types::CheckpointEntry;

use crate::{define_table_with_integer_key, define_table_without_codec, impl_borsh_value_codec};

define_table_with_integer_key!(
    /// A table to store idx -> `CheckpointEntry` mapping
    (CheckpointSchema) u64 => CheckpointEntry
);

define_table_with_integer_key!(
    /// Table mapping epoch indexes to the list of summaries in that index.
    (EpochSummarySchema) u64 => Vec<EpochSummary>
);

define_table_with_integer_key!(
    /// Tracks checkpoints that still require proof generation.
    (PendingProofIndexSchema) u64 => ()
);
