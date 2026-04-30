use strata_asm_proto_checkpoint_types::CheckpointPayload;
use strata_checkpoint_types::EpochSummary;
use strata_csm_types::CheckpointL1Ref;
use strata_db_types::types::L1PayloadIntentIndex;
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
    /// Table mapping epoch commitment to persisted [`CheckpointL1Ref`].
    (OLCheckpointL1RefSchema) EpochCommitment => CheckpointL1Ref
);

define_table_with_integer_key!(
    /// Presence marker: this epoch has at least one unsigned payload.
    (UnsignedCheckpointIndexSchema) Epoch => ()
);

define_table_with_integer_key!(
    /// Table mapping epoch indexes to the list of summaries in that index.
    (OLEpochSummarySchema) u64 => Vec<EpochSummary>
);
