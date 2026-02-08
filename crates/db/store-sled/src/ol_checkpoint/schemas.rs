use strata_checkpoint_types::EpochSummary;
use strata_db_types::types::OLCheckpointEntry;
use strata_identifiers::Epoch;

use crate::define_table_with_integer_key;

define_table_with_integer_key!(
    /// Table mapping epoch to OL checkpoint entry.
    (OLCheckpointSchema) Epoch => OLCheckpointEntry
);

define_table_with_integer_key!(
    /// Tracks checkpoints that are unsigned.
    (UnsignedCheckpointIndexSchema) Epoch => ()
);

define_table_with_integer_key!(
    /// Table mapping epoch indexes to the list of summaries in that index.
    (OLEpochSummarySchema) u64 => Vec<EpochSummary>
);
