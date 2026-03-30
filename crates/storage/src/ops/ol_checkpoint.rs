use strata_checkpoint_types::EpochSummary;
use strata_checkpoint_types_ssz::CheckpointPayload;
use strata_db_types::{
    traits::OLCheckpointDatabase,
    types::{L1PayloadIntentIndex, OLCheckpointL1ObservationEntry},
};
use strata_identifiers::Epoch;
use strata_primitives::epoch::EpochCommitment;

use crate::{exec::*, instrumentation::components};

inst_ops_simple! {
    (<D: OLCheckpointDatabase> => OLCheckpointOps, component = components::STORAGE_OL_CHECKPOINT) {
        insert_epoch_summary(summary: EpochSummary) => ();
        get_epoch_summary(epoch: EpochCommitment) => Option<EpochSummary>;
        get_epoch_commitments_at(epoch: Epoch) => Vec<EpochCommitment>;
        get_last_summarized_epoch() => Option<Epoch>;
        del_epoch_summary(epoch: EpochCommitment) => bool;
        del_epoch_summaries_from_epoch(start_epoch: Epoch) => Vec<EpochCommitment>;
        put_checkpoint_payload_entry(epoch: EpochCommitment, payload: CheckpointPayload) => ();
        get_checkpoint_payload_entry(epoch: EpochCommitment) => Option<CheckpointPayload>;
        get_last_checkpoint_payload_epoch() => Option<EpochCommitment>;
        del_checkpoint_payload_entry(epoch: EpochCommitment) => bool;
        del_checkpoint_payload_entries_from_epoch(start_epoch: Epoch) => Vec<EpochCommitment>;
        put_checkpoint_signing_entry(epoch: EpochCommitment, payload_intent_idx: L1PayloadIntentIndex) => ();
        get_checkpoint_signing_entry(epoch: EpochCommitment) => Option<L1PayloadIntentIndex>;
        del_checkpoint_signing_entry(epoch: EpochCommitment) => bool;
        del_checkpoint_signing_entries_from_epoch(start_epoch: Epoch) => Vec<EpochCommitment>;
        put_checkpoint_l1_observation_entry(epoch: EpochCommitment, l1_observation: OLCheckpointL1ObservationEntry) => ();
        get_checkpoint_l1_observation_entry(epoch: EpochCommitment) => Option<OLCheckpointL1ObservationEntry>;
        del_checkpoint_l1_observation_entry(epoch: EpochCommitment) => bool;
        del_checkpoint_l1_observation_entries_from_epoch(start_epoch: Epoch) => Vec<EpochCommitment>;
        get_next_unsigned_checkpoint_epoch() => Option<Epoch>;
    }
}
