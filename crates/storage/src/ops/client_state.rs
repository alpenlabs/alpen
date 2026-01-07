//! Client data database operations interface..

use strata_common::instrumentation::components;
use strata_csm_types::{ClientState, ClientUpdateOutput};
use strata_db_types::traits::*;
use strata_primitives::l1::L1BlockCommitment;

use crate::exec::*;

inst_ops_simple! {
    (<D: ClientStateDatabase> => ClientStateOps, component = components::STORAGE_CLIENT_STATE) {
        put_client_update(block: L1BlockCommitment, output: ClientUpdateOutput) => ();
        get_client_update(block: L1BlockCommitment) => Option<ClientUpdateOutput>;
        get_latest_client_state() => Option<(L1BlockCommitment, ClientState)>;
        del_client_update(block: L1BlockCommitment) => ();
        get_client_updates_from(from_block: L1BlockCommitment, max_count: usize) => Vec<(L1BlockCommitment, ClientUpdateOutput)>;
    }
}
