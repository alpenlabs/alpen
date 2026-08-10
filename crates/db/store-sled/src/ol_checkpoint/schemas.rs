use strata_asm_checkpoint_types::CheckpointPayload;
use strata_checkpoint_types::EpochSummary;
use strata_csm_types::CheckpointL1Ref;
use strata_db_types::common::L1PayloadIntentIndex;
use strata_identifiers::{Epoch, EpochCommitment};

use crate::{
    define_table_without_codec, impl_cbor_value_codec, impl_codec_key_codec, impl_ssz_value_codec,
};

// `EpochCommitment` keys encode big-endian as `epoch(4) || last_slot(8) || blkid(32)`, so
// lexicographic tree order matches epoch order and the checkpoint queries can range-seek.

define_table_without_codec!(
    /// Table mapping epoch commitment to OL checkpoint payload.
    (OLCheckpointPayloadSchema) EpochCommitment => CheckpointPayload
);
impl_codec_key_codec!(OLCheckpointPayloadSchema, EpochCommitment);
impl_ssz_value_codec!(OLCheckpointPayloadSchema, CheckpointPayload);

define_table_without_codec!(
    /// Table mapping epoch commitment to the OL checkpoint payload extracted
    /// from L1, kept separate from the sequencer's locally-built payload table.
    (OLCheckpointL1ObservedPayloadSchema) EpochCommitment => CheckpointPayload
);
impl_codec_key_codec!(OLCheckpointL1ObservedPayloadSchema, EpochCommitment);
impl_ssz_value_codec!(OLCheckpointL1ObservedPayloadSchema, CheckpointPayload);

define_table_without_codec!(
    /// Table mapping epoch to OL checkpoint payload intent index.
    (OLCheckpointSigningSchema) EpochCommitment => L1PayloadIntentIndex
);
impl_codec_key_codec!(OLCheckpointSigningSchema, EpochCommitment);
impl_cbor_value_codec!(OLCheckpointSigningSchema, L1PayloadIntentIndex);

define_table_without_codec!(
    /// Table mapping epoch commitment to persisted [`CheckpointL1Ref`].
    (OLCheckpointL1RefSchema) EpochCommitment => CheckpointL1Ref
);
impl_codec_key_codec!(OLCheckpointL1RefSchema, EpochCommitment);
impl_cbor_value_codec!(OLCheckpointL1RefSchema, CheckpointL1Ref);

define_table_without_codec!(
    /// Presence marker: this epoch has at least one unsigned payload.
    (UnsignedCheckpointIndexSchema) Epoch => ()
);
// `Epoch` is a `u32` alias and uses typed-sled's blanket big-endian integer key codec.
impl_cbor_value_codec!(UnsignedCheckpointIndexSchema, ());

define_table_without_codec!(
    /// Table mapping epoch indexes to the list of summaries in that index.
    (OLEpochSummarySchema) u64 => Vec<EpochSummary>
);
impl_cbor_value_codec!(OLEpochSummarySchema, Vec<EpochSummary>);

define_table_without_codec!(
    /// Observed checkpoint commitments per epoch number.
    ///
    /// Observed candidate set: reorged observations remain until explicit
    /// pruning, so canonicity is resolved at read time.
    (OLCheckpointEpochIndexSchema) Epoch => Vec<EpochCommitment>
);
impl_cbor_value_codec!(OLCheckpointEpochIndexSchema, Vec<EpochCommitment>);
