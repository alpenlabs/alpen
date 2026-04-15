//! Prover task database operation interface.

use strata_db_types::traits::ProverTaskDatabase;
use strata_paas::TaskRecordData;

use crate::{exec::*, instrumentation::components};

inst_ops_simple! {
    (<D: ProverTaskDatabase> => ProverTaskDbOps, component = components::STORAGE_PROVER_TASK) {
        get_task(key: Vec<u8>) => Option<TaskRecordData>;
        insert_task(key: Vec<u8>, record: TaskRecordData) => ();
        put_task(key: Vec<u8>, record: TaskRecordData) => ();
        list_retriable(now_secs: u64) => Vec<(Vec<u8>, TaskRecordData)>;
        list_unfinished() => Vec<(Vec<u8>, TaskRecordData)>;
        count_tasks() => usize;
    }
}
