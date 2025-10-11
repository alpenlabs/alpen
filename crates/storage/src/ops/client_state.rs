//! Client data database operations interface..

use std::sync::Arc;

use strata_csm_types::{ClientState, ClientUpdateOutput};
use strata_db::traits::*;
use strata_primitives::l1::L1BlockCommitment;

use crate::exec::*;

inst_ops_simple! {
    (<D: ClientStateDatabase> => ClientStateOps) {
        put_client_update(block: L1BlockCommitment, output: ClientUpdateOutput) => ();
        get_client_update(block: L1BlockCommitment) => Option<ClientUpdateOutput>;
        get_latest_client_state() => Option<(L1BlockCommitment, ClientState)>;
    }
}
