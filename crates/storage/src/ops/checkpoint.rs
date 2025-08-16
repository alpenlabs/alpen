//! Checkpoint Proof data operation interface.

use std::sync::Arc;

use strata_db::{traits::*, types::CheckpointEntry};
use strata_primitives::epoch::EpochCommitment;
use strata_state::batch::EpochSummary;

use crate::exec::*;

inst_ops_simple! {
    (<D: CheckpointDatabase> => CheckpointDataOps) {
        insert_epoch_summary(epoch: EpochSummary) => ();
        get_epoch_summary(epoch: EpochCommitment) => Option<EpochSummary>;
        get_epoch_commitments_at(epoch: u64) => Vec<EpochCommitment>;
        get_last_summarized_epoch() => Option<u64>;
        put_checkpoint(idx: u64, entry: CheckpointEntry) => ();
        get_checkpoint(idx: u64) => Option<CheckpointEntry>;
        get_last_checkpoint_idx() => Option<u64>;
        get_latest_unproven_checkpoint_idx() => Option<u64>;
    }
}
