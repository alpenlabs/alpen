//! Client data database operations interface..

use std::sync::Arc;

use strata_db::traits::*;
use strata_state::{client_state::L1ClientState, l1::L1BlockId, operation::ClientUpdateOutput};

use crate::exec::*;

inst_ops_simple! {
    (<D: ClientStateDatabase> => ClientStateOps) {
        put_client_update(idx: u64, output: ClientUpdateOutput) => ();
        get_client_update(idx: u64) => Option<ClientUpdateOutput>;
        get_last_state_idx() => u64;

        put_client_state(block_id: L1BlockId, state: L1ClientState) => ();
        get_client_state(block_id: L1BlockId) => Option<L1ClientState>;
    }
}
