use strata_db_types::traits::OLStateDatabase;
use strata_identifiers::OLBlockCommitment;
use strata_ol_state_types::{NativeAccountState, OLState, WriteBatch};

use crate::{exec::*, instrumentation::components};

inst_ops_simple! {
    (<D: OLStateDatabase> => OLStateOps, component = components::STORAGE_OL_STATE) {
        put_toplevel_ol_state(commitment: OLBlockCommitment, state: OLState) => ();
        get_toplevel_ol_state(commitment: OLBlockCommitment) => Option<OLState>;
        get_latest_toplevel_ol_state() => Option<(OLBlockCommitment, OLState)>;
        del_toplevel_ol_state(commitment: OLBlockCommitment) => ();
        put_ol_write_batch(commitment: OLBlockCommitment, wb: WriteBatch<NativeAccountState>) => ();
        get_ol_write_batch(commitment: OLBlockCommitment) => Option<WriteBatch<NativeAccountState>>;
        del_ol_write_batch(commitment: OLBlockCommitment) => ();
    }
}
