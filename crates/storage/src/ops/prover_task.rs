//! Prover task database operation interface.

use strata_db_types::{
    traits::ProverTaskDatabase,
    types::{SerializableTaskId, SerializableTaskRecord},
};

use crate::{exec::*, instrumentation::components};

inst_ops_simple! {
    (<D: ProverTaskDatabase> => ProverTaskDbOps, component = components::STORAGE_PROVER_TASK) {
        get_task(task_id: SerializableTaskId) => Option<SerializableTaskRecord>;
        get_task_id_by_uuid(uuid: String) => Option<SerializableTaskId>;
        insert_task(task_id: SerializableTaskId, record: SerializableTaskRecord) => ();
        update_task(task_id: SerializableTaskId, record: SerializableTaskRecord) => ();
        list_all_tasks() => Vec<(SerializableTaskId, SerializableTaskRecord)>;
    }
}
