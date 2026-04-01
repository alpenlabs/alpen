//! High-level manager for prover task record database access.

use std::sync::Arc;

use strata_db_types::{
    traits::ProverTaskDatabase,
    types::{SerializableTaskId, SerializableTaskRecord},
    DbResult,
};
use threadpool::ThreadPool;

use crate::ops::prover_task::{Context, ProverTaskDbOps};

#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct ProverTaskDbManager {
    ops: ProverTaskDbOps,
}

impl ProverTaskDbManager {
    pub fn new(pool: ThreadPool, db: Arc<impl ProverTaskDatabase + 'static>) -> Self {
        let ops = Context::new(db).into_ops(pool);
        Self { ops }
    }

    pub fn get_task(
        &self,
        task_id: SerializableTaskId,
    ) -> DbResult<Option<SerializableTaskRecord>> {
        self.ops.get_task_blocking(task_id)
    }

    pub fn get_task_id_by_uuid(&self, uuid: String) -> DbResult<Option<SerializableTaskId>> {
        self.ops.get_task_id_by_uuid_blocking(uuid)
    }

    pub fn insert_task(
        &self,
        task_id: SerializableTaskId,
        record: SerializableTaskRecord,
    ) -> DbResult<()> {
        self.ops.insert_task_blocking(task_id, record)
    }

    pub fn update_task(
        &self,
        task_id: SerializableTaskId,
        record: SerializableTaskRecord,
    ) -> DbResult<()> {
        self.ops.update_task_blocking(task_id, record)
    }

    pub fn list_all_tasks(&self) -> DbResult<Vec<(SerializableTaskId, SerializableTaskRecord)>> {
        self.ops.list_all_tasks_blocking()
    }
}
