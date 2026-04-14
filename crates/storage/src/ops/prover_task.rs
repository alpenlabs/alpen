//! Prover task database operation interface.

use strata_db_types::{traits::ProverTaskDatabase, types::PersistedTaskRecord};

use crate::{exec::*, instrumentation::components};

inst_ops_simple! {
    (<D: ProverTaskDatabase> => ProverTaskDbOps, component = components::STORAGE_PROVER_TASK) {
        get_task(key: Vec<u8>) => Option<PersistedTaskRecord>;
        insert_task(key: Vec<u8>, record: PersistedTaskRecord) => ();
        put_task(key: Vec<u8>, record: PersistedTaskRecord) => ();
        list_retriable(now_secs: u64) => Vec<(Vec<u8>, PersistedTaskRecord)>;
        list_unfinished() => Vec<(Vec<u8>, PersistedTaskRecord)>;
        count_tasks() => usize;
    }
}
