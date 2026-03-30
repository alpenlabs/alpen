use strata_checkpoint_types::EpochSummary;
use strata_checkpoint_types_ssz::CheckpointPayload;
use strata_db_types::types::{L1PayloadIntentIndex, OLCheckpointL1ObservationEntry};
use strata_identifiers::{Epoch, EpochCommitment};

use crate::{define_table_with_default_codec, define_table_with_integer_key};

define_table_with_default_codec!(
    /// Table mapping epoch commitment to OL checkpoint payload.
    (OLCheckpointPayloadSchema) EpochCommitment => CheckpointPayload
);

define_table_with_default_codec!(
    /// Table mapping epoch to OL checkpoint payload intent index.
    (OLCheckpointSigningSchema) EpochCommitment => L1PayloadIntentIndex
);

define_table_with_default_codec!(
    /// Table mapping epoch commitment to observed L1 height for checkpoint tip update.
    (OLCheckpointL1ObservationSchema) EpochCommitment => OLCheckpointL1ObservationEntry
);

define_table_with_integer_key!(
    /// Presence marker: this epoch has at least one unsigned payload.
    (UnsignedCheckpointIndexSchema) Epoch => ()
);

define_table_with_integer_key!(
    /// Table mapping epoch indexes to the list of summaries in that index.
    (OLEpochSummarySchema) u64 => Vec<EpochSummary>
);
