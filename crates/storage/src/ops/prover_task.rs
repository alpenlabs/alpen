//! Prover task database operation interface.

use strata_db_types::{
    traits::ProverTaskDatabase,
    types::{PersistedTaskId, PersistedTaskRecord},
};

use crate::{exec::*, instrumentation::components};

inst_ops_simple! {
    (<D: ProverTaskDatabase> => ProverTaskDbOps, component = components::STORAGE_PROVER_TASK) {
        get_task(task_id: PersistedTaskId) => Option<PersistedTaskRecord>;
        get_task_id_by_uuid(uuid: String) => Option<PersistedTaskId>;
        insert_task(task_id: PersistedTaskId, record: PersistedTaskRecord) => ();
        update_task(task_id: PersistedTaskId, record: PersistedTaskRecord) => ();
        list_all_tasks() => Vec<(PersistedTaskId, PersistedTaskRecord)>;
    }
}
