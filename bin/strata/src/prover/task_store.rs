//! Persistent [`TaskStore`] for the integrated prover, backed by storage manager.
//!
//! Reuses the existing paas task tracking schema in SledDB so that task state
//! survives restarts. The epoch runner is idempotent, but persistence lets paas
//! detect duplicate submissions and resume in-progress tasks.

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use strata_db_types::types::{PersistedTaskId, PersistedTaskRecord};
use strata_paas::{
    ProverServiceError, ProverServiceResult, TaskId, TaskRecord, TaskStatus, TaskStore, ZkVmBackend,
};
use strata_primitives::proof::ProofContext;
use strata_storage::ProverTaskDbManager;

use super::task::CheckpointTask;

/// Persistent task store for checkpoint proof tasks.
///
/// Maps [`CheckpointTask`] to the existing [`PersistedTaskId`] schema
/// via commitment-aware proof context keys, sharing the same database instance
/// as the proof storer.
pub(crate) struct PersistentTaskStore {
    db: Arc<ProverTaskDbManager>,
}

impl PersistentTaskStore {
    pub(crate) fn new(db: Arc<ProverTaskDbManager>) -> Self {
        Self { db }
    }

    fn to_persisted_id(task_id: &TaskId<CheckpointTask>) -> PersistedTaskId {
        PersistedTaskId {
            program: ProofContext::CheckpointCommitment(task_id.program().commitment),
            backend: match task_id.backend() {
                ZkVmBackend::Native => 0,
                ZkVmBackend::SP1 => 1,
                ZkVmBackend::Risc0 => 2,
            },
        }
    }

    fn from_persisted_id(ser: &PersistedTaskId) -> ProverServiceResult<TaskId<CheckpointTask>> {
        let backend = match ser.backend {
            0 => ZkVmBackend::Native,
            1 => ZkVmBackend::SP1,
            2 => ZkVmBackend::Risc0,
            other => {
                return Err(ProverServiceError::Internal(anyhow::anyhow!(
                    "unknown backend discriminant: {other}"
                )));
            }
        };
        let commitment = match ser.program {
            ProofContext::CheckpointCommitment(commitment) => commitment,
            ref other => {
                return Err(ProverServiceError::Internal(anyhow::anyhow!(
                    "unexpected proof context for checkpoint task store: {other:?}"
                )));
            }
        };
        Ok(TaskId::new(
            CheckpointTask::new(commitment, backend.clone()),
            backend,
        ))
    }

    fn to_persisted_record(record: &TaskRecord<TaskId<CheckpointTask>>) -> PersistedTaskRecord {
        PersistedTaskRecord {
            task_id: Self::to_persisted_id(record.task_id()),
            uuid: record.uuid().to_string(),
            status: record.status().clone(),
            created_at_secs: now_secs(),
            updated_at_secs: now_secs(),
        }
    }

    fn from_persisted_record(
        ser: &PersistedTaskRecord,
    ) -> ProverServiceResult<TaskRecord<TaskId<CheckpointTask>>> {
        Ok(TaskRecord::new(
            Self::from_persisted_id(&ser.task_id)?,
            ser.uuid.clone(),
            ser.status.clone(),
        ))
    }
}

impl TaskStore<CheckpointTask> for PersistentTaskStore {
    fn get_uuid(&self, task_id: &TaskId<CheckpointTask>) -> Option<String> {
        let key = Self::to_persisted_id(task_id);
        self.db.get_task(key).ok()?.map(|r| r.uuid)
    }

    fn get_task(
        &self,
        task_id: &TaskId<CheckpointTask>,
    ) -> Option<TaskRecord<TaskId<CheckpointTask>>> {
        let key = Self::to_persisted_id(task_id);
        let ser = self.db.get_task(key).ok()??;
        Self::from_persisted_record(&ser).ok()
    }

    fn get_task_by_uuid(&self, uuid: &str) -> Option<TaskRecord<TaskId<CheckpointTask>>> {
        let task_id_ser = self.db.get_task_id_by_uuid(uuid.to_string()).ok()??;
        let record = self.db.get_task(task_id_ser).ok()??;
        Self::from_persisted_record(&record).ok()
    }

    fn insert_task(&self, record: TaskRecord<TaskId<CheckpointTask>>) -> ProverServiceResult<()> {
        let key = Self::to_persisted_id(record.task_id());

        if self
            .db
            .get_task(key.clone())
            .map_err(|e| ProverServiceError::Internal(anyhow::anyhow!("db error: {e}")))?
            .is_some()
        {
            return Err(ProverServiceError::Config(format!(
                "task already exists: {:?}",
                record.task_id()
            )));
        }

        let value = Self::to_persisted_record(&record);
        self.db
            .insert_task(key, value)
            .map_err(|e| ProverServiceError::Internal(anyhow::anyhow!("insert failed: {e}")))?;
        Ok(())
    }

    fn update_status(
        &self,
        task_id: &TaskId<CheckpointTask>,
        status: TaskStatus,
    ) -> ProverServiceResult<()> {
        let key = Self::to_persisted_id(task_id);

        let mut record = self
            .db
            .get_task(key.clone())
            .map_err(|e| ProverServiceError::Internal(anyhow::anyhow!("db error: {e}")))?
            .ok_or_else(|| {
                ProverServiceError::TaskNotFound(format!("task not found: {task_id:?}"))
            })?;

        record.status = status;
        record.updated_at_secs = now_secs();

        self.db
            .update_task(key, record)
            .map_err(|e| ProverServiceError::Internal(anyhow::anyhow!("update failed: {e}")))?;
        Ok(())
    }

    fn list_tasks(
        &self,
        filter: Box<dyn Fn(&TaskStatus) -> bool + '_>,
    ) -> Vec<TaskRecord<TaskId<CheckpointTask>>> {
        self.db
            .list_all_tasks()
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, record)| filter(&record.status))
            .filter_map(|(_, record)| Self::from_persisted_record(&record).ok())
            .collect()
    }

    fn count(&self) -> usize {
        self.db
            .list_all_tasks()
            .map(|records| records.len())
            .unwrap_or_default()
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
