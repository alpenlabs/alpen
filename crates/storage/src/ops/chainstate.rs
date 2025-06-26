//! Low level database ops for chainstate database.

use std::sync::Arc;

use strata_db::traits::*;
use strata_state::{chain_state::Chainstate, id::L2BlockId, state_op::WriteBatchEntry};

use crate::exec::*;

inst_ops_simple! {
    (<D: OldChainstateDatabase> => OldChainstateOps) {
        write_genesis_state(toplevel: Chainstate, blockid: L2BlockId) => ();
        put_write_batch(idx: u64, batch: WriteBatchEntry) => ();
        get_write_batch(idx: u64) => Option<WriteBatchEntry>;
        purge_entries_before(before_idx: u64) => ();
        rollback_writes_to(new_tip_idx: u64) => ();
        get_last_write_idx() => u64;
        get_earliest_write_idx() => u64;
    }
}
