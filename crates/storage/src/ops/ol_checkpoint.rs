use strata_checkpoint_types::EpochSummary;
use strata_db_types::{traits::OLCheckpointDatabase, types::OLCheckpointEntry};
use strata_identifiers::Epoch;
use strata_primitives::epoch::EpochCommitment;

use crate::{exec::*, instrumentation::components};

inst_ops_simple! {
    (<D: OLCheckpointDatabase> => OLCheckpointOps, component = components::STORAGE_OL_CHECKPOINT) {
        insert_epoch_summary(summary: EpochSummary) => ();
        get_epoch_summary(epoch: EpochCommitment) => Option<EpochSummary>;
        get_epoch_commitments_at(epoch: u64) => Vec<EpochCommitment>;
        get_last_summarized_epoch() => Option<u64>;
        del_epoch_summary(epoch: EpochCommitment) => bool;
        del_epoch_summaries_from_epoch(start_epoch: u64) => Vec<u64>;
        put_checkpoint(epoch: Epoch, entry: OLCheckpointEntry) => ();
        get_checkpoint(epoch: Epoch) => Option<OLCheckpointEntry>;
        get_last_checkpoint_epoch() => Option<Epoch>;
        get_next_unsigned_checkpoint_epoch() => Option<Epoch>;
        del_checkpoint(epoch: Epoch) => bool;
        del_checkpoints_from_epoch(start_epoch: Epoch) => Vec<Epoch>;
    }
}
