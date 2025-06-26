use std::sync::Arc;

use strata_db::chainstate::*;
use strata_primitives::buf::Buf32;
use strata_state::{chain_state::Chainstate, state_op::WriteBatch};

use crate::exec::*;

inst_ops_simple! {
    (<D: ChainstateDatabase> => ChainstateOps) {
        create_new_inst(toplevel: Chainstate) => StateInstanceId;
        clone_inst(id: StateInstanceId) => StateInstanceId;
        del_inst(id: StateInstanceId) => ();
        get_insts() => Vec<StateInstanceId>;
        get_inst_root(id: StateInstanceId) => Buf32;
        get_inst_toplevel_state(id: StateInstanceId) => Chainstate;
        put_write_batch(id: WriteBatchId, wb: WriteBatch) => ();
        get_write_batch(id: WriteBatchId) => Option<WriteBatch>;
        del_write_batch(id: WriteBatchId) => ();
        merge_write_batches(state_id: StateInstanceId, wb_ids: Vec<WriteBatchId>) => ();
    }
}
