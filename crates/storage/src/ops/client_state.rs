//! Client data database operations interface..

use std::sync::Arc;

use strata_db::traits::*;
use strata_primitives::l1::L1BlockCommitment;
use strata_state::{client_state::ClientState, operation::ClientUpdateOutput};

use crate::exec::*;

inst_ops_simple! {
    (<D: ClientStateDatabase> => ClientStateOps) {
        put_client_update(block: L1BlockCommitment, output: ClientUpdateOutput) => ();
        get_client_update(block: L1BlockCommitment) => Option<ClientUpdateOutput>;
        get_latest_client_state() => Option<(L1BlockCommitment, ClientState)>;
        get_client_states_at_height(height: u64) => Vec<ClientState>;
    }
}
