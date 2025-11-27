//! TaskStore implementation using ProofDBSled

use std::{
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use strata_db_store_sled::prover::{ProofDBSled, SerializableTaskId, SerializableTaskRecord};
use strata_paas::{
    ProverServiceError, ProverServiceResult, TaskId, TaskRecord, TaskStatus, TaskStore,
};

use super::ProofTask;

/// TaskStore implementation backed by ProofDBSled
#[derive(Clone)]
pub(crate) struct SledTaskStore {
    db: Arc<ProofDBSled>,
}

impl SledTaskStore {
    pub(crate) fn new(db: Arc<ProofDBSled>) -> Self {
        Self { db }
    }

    /// Convert from paas TaskId to serializable form
    fn to_serializable_task_id(task_id: &TaskId<ProofTask>) -> SerializableTaskId {
        SerializableTaskId {
            program: task_id.program().0, // ProofTask wraps ProofContext
            backend: match task_id.backend() {
                strata_paas::ZkVmBackend::Native => 0,
                strata_paas::ZkVmBackend::SP1 => 1,
                strata_paas::ZkVmBackend::Risc0 => 2,
            },
        }
    }

    /// Convert from serializable form to paas TaskId
    fn from_serializable_task_id(ser: &SerializableTaskId) -> TaskId<ProofTask> {
        let backend = match ser.backend {
            0 => strata_paas::ZkVmBackend::Native,
            1 => strata_paas::ZkVmBackend::SP1,
            2 => strata_paas::ZkVmBackend::Risc0,
            _ => strata_paas::ZkVmBackend::Native, // Default fallback
        };
        TaskId::new(ProofTask(ser.program), backend)
    }

    /// Convert SystemTime to Instant (approximate)
    fn system_time_to_instant(_secs: u64) -> Instant {
        // For persistence, we lose the exact instant but that's OK for task tracking
        // We just use "now" as the reference point
        Instant::now()
    }

    /// Convert Instant to SystemTime seconds
    fn instant_to_secs(_instant: &Instant) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    /// Convert from paas TaskRecord to serializable form
    fn to_serializable_record(record: &TaskRecord<TaskId<ProofTask>>) -> SerializableTaskRecord {
        SerializableTaskRecord {
            task_id: Self::to_serializable_task_id(&record.task_id),
            uuid: record.uuid.clone(),
            status: record.status.clone(),
            created_at_secs: Self::instant_to_secs(&record.created_at),
            updated_at_secs: Self::instant_to_secs(&record.updated_at),
        }
    }

    /// Convert from serializable form to paas TaskRecord
    fn from_serializable_record(ser: &SerializableTaskRecord) -> TaskRecord<TaskId<ProofTask>> {
        TaskRecord {
            task_id: Self::from_serializable_task_id(&ser.task_id),
            uuid: ser.uuid.clone(),
            status: ser.status.clone(),
            created_at: Self::system_time_to_instant(ser.created_at_secs),
            updated_at: Self::system_time_to_instant(ser.updated_at_secs),
        }
    }
}

impl TaskStore<ProofTask> for SledTaskStore {
    fn get_uuid(&self, task_id: &TaskId<ProofTask>) -> Option<String> {
        let key = Self::to_serializable_task_id(task_id);
        let record = self.db.get_task(&key).ok()??;
        Some(record.uuid)
    }

    fn get_task(&self, task_id: &TaskId<ProofTask>) -> Option<TaskRecord<TaskId<ProofTask>>> {
        let key = Self::to_serializable_task_id(task_id);
        let record = self.db.get_task(&key).ok()??;
        Some(Self::from_serializable_record(&record))
    }

    fn get_task_by_uuid(&self, uuid: &str) -> Option<TaskRecord<TaskId<ProofTask>>> {
        // Look up TaskId from UUID index
        let task_id_ser = self.db.get_task_id_by_uuid(uuid).ok()??;
        // Get full record from main tree
        let record = self.db.get_task(&task_id_ser).ok()??;
        Some(Self::from_serializable_record(&record))
    }

    fn insert_task(&self, record: TaskRecord<TaskId<ProofTask>>) -> ProverServiceResult<()> {
        let key = Self::to_serializable_task_id(&record.task_id);

        // Check for duplicate task_id
        if self
            .db
            .get_task(&key)
            .map_err(|e| ProverServiceError::Internal(anyhow::anyhow!("DB error: {}", e)))?
            .is_some()
        {
            return Err(ProverServiceError::Config(format!(
                "Task already exists: {:?}",
                record.task_id
            )));
        }

        // Check for duplicate UUID
        if self
            .db
            .get_task_id_by_uuid(&record.uuid)
            .map_err(|e| ProverServiceError::Internal(anyhow::anyhow!("DB error: {}", e)))?
            .is_some()
        {
            return Err(ProverServiceError::Internal(anyhow::anyhow!(
                "UUID collision detected: {}",
                record.uuid
            )));
        }

        let value = Self::to_serializable_record(&record);

        // Insert into both trees
        self.db.insert_task(&key, &value).map_err(|e| {
            ProverServiceError::Internal(anyhow::anyhow!("Failed to insert task: {}", e))
        })?;

        Ok(())
    }

    fn update_status(
        &self,
        task_id: &TaskId<ProofTask>,
        status: TaskStatus,
    ) -> ProverServiceResult<()> {
        let key = Self::to_serializable_task_id(task_id);

        // Get existing record
        let mut record = self
            .db
            .get_task(&key)
            .map_err(|e| ProverServiceError::Internal(anyhow::anyhow!("DB error: {}", e)))?
            .ok_or_else(|| {
                ProverServiceError::TaskNotFound(format!("Task not found: {:?}", task_id))
            })?;

        // Update status and timestamp
        record.status = status;
        record.updated_at_secs = Self::instant_to_secs(&Instant::now());

        // Write back
        self.db.update_task(&key, &record).map_err(|e| {
            ProverServiceError::Internal(anyhow::anyhow!("Failed to update task: {}", e))
        })?;

        Ok(())
    }

    fn list_tasks(
        &self,
        filter: Box<dyn Fn(&TaskStatus) -> bool + '_>,
    ) -> Vec<TaskRecord<TaskId<ProofTask>>> {
        self.db
            .list_all_tasks()
            .into_iter()
            .filter(|(_key, record)| filter(&record.status))
            .map(|(_key, record)| Self::from_serializable_record(&record))
            .collect()
    }

    fn count(&self) -> usize {
        // Count all tasks by listing with a filter that accepts everything
        self.list_tasks(Box::new(|_| true)).len()
    }
}
