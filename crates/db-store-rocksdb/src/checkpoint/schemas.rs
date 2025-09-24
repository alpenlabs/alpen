use strata_checkpoint_types::EpochSummary;
use strata_db::types::CheckpointEntry;

use crate::{define_table_with_seek_key_codec, define_table_without_codec, impl_borsh_value_codec};

define_table_with_seek_key_codec!(
    /// A table to store idx -> `CheckpointEntry` mapping
    (CheckpointSchema) u64 => CheckpointEntry
);

define_table_with_seek_key_codec!(
    /// Table mapping epoch indexes to the list of summaries in that index.
    // This is weird because I wanted to avoid too much duplication and
    // `EpochCommitment` doesn't implement `KeyCodec`.
    // TODO figure out a better solution, the act of fetching an epoch's
    // commitments and then iterating over the summaries in it is really slow
    // the way it is now, maybe split apart the "inner" part from the
    // identifying part?
    (EpochSummarySchema) u64 => Vec<EpochSummary>
);
