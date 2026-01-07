//! Checkpoint Proof data operation interface.

use strata_checkpoint_types::EpochSummary;
use strata_common::instrumentation::components;
use strata_db_types::{traits::*, types::CheckpointEntry};
use strata_primitives::epoch::EpochCommitment;

use crate::exec::*;

inst_ops_simple! {
    (<D: CheckpointDatabase> => CheckpointDataOps, component = components::STORAGE_CHECKPOINT) {
        insert_epoch_summary(epoch: EpochSummary) => ();
        get_epoch_summary(epoch: EpochCommitment) => Option<EpochSummary>;
        get_epoch_commitments_at(epoch: u64) => Vec<EpochCommitment>;
        get_last_summarized_epoch() => Option<u64>;
        put_checkpoint(idx: u64, entry: CheckpointEntry) => ();
        get_checkpoint(idx: u64) => Option<CheckpointEntry>;
        get_last_checkpoint_idx() => Option<u64>;
        get_next_unproven_checkpoint_idx() => Option<u64>;
    }
}
