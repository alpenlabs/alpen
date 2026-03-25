//! Sled-backed [`TaskStore`] for EE proof tasks.
//!
//! Guarantees idempotency across restarts and preserves SP1 request state
//! so we can resume polling instead of re-submitting.

use std::time::{SystemTime, UNIX_EPOCH};

use alpen_ee_common::EeProofTask;
use strata_paas::{
    ProverServiceError, ProverServiceResult, TaskId, TaskRecord, TaskStatus, TaskStore,
};

/// Serializable form of `EeProofTask` for sled storage.
///
/// Uses borsh for deterministic encoding compatible with sled trees.
#[derive(Debug, Clone, PartialEq, Eq, Hash, borsh::BorshSerialize, borsh::BorshDeserialize)]
struct EeSerializableTaskId {
    /// Borsh-encoded `EeProofTask`.
    task_bytes: Vec<u8>,
    /// Backend: 0=Native, 1=SP1, 2=Risc0.
    backend: u8,
}

/// Serializable task record for sled storage.
#[derive(Debug, Clone, borsh::BorshSerialize, borsh::BorshDeserialize)]
struct EeSerializableTaskRecord {
    task_id: EeSerializableTaskId,
    uuid: String,
    status: TaskStatus,
    created_at_secs: u64,
    updated_at_secs: u64,
}

fn backend_to_u8(backend: &strata_paas::ZkVmBackend) -> u8 {
    match backend {
        strata_paas::ZkVmBackend::Native => 0,
        strata_paas::ZkVmBackend::SP1 => 1,
        strata_paas::ZkVmBackend::Risc0 => 2,
    }
}

fn u8_to_backend(b: u8) -> strata_paas::ZkVmBackend {
    match b {
        0 => strata_paas::ZkVmBackend::Native,
        1 => strata_paas::ZkVmBackend::SP1,
        2 => strata_paas::ZkVmBackend::Risc0,
        _ => strata_paas::ZkVmBackend::Native,
    }
}

fn to_sled_task_id(task_id: &TaskId<EeProofTask>) -> EeSerializableTaskId {
    let task_bytes =
        serde_json::to_vec(task_id.program()).expect("EeProofTask serialization should not fail");
    EeSerializableTaskId {
        task_bytes,
        backend: backend_to_u8(task_id.backend()),
    }
}

fn from_sled_task_id(ser: &EeSerializableTaskId) -> TaskId<EeProofTask> {
    let program: EeProofTask =
        serde_json::from_slice(&ser.task_bytes).expect("EeProofTask deserialization should not fail");
    TaskId::new(program, u8_to_backend(ser.backend))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Persistent task store backed by sled.
///
/// Persists task records so that on restart we can:
/// - Detect already-submitted tasks (idempotency).
/// - Resume polling for in-flight SP1 proofs.
/// - Avoid re-submitting paid remote proving requests.
///
/// Uses two sled trees:
/// - `ee_paas_tasks`: `EeSerializableTaskId -> EeSerializableTaskRecord`
/// - `ee_paas_uuid_idx`: `UUID (String) -> EeSerializableTaskId`
pub(crate) struct SledTaskStore {
    task_tree: sled::Tree,
    uuid_tree: sled::Tree,
}

impl SledTaskStore {
    /// Opens (or creates) the EE proof task trees in the given sled database.
    pub(crate) fn new(db: &sled::Db) -> eyre::Result<Self> {
        let task_tree = db.open_tree("ee_paas_tasks")?;
        let uuid_tree = db.open_tree("ee_paas_uuid_idx")?;
        Ok(Self {
            task_tree,
            uuid_tree,
        })
    }

    fn ser_key(id: &EeSerializableTaskId) -> Vec<u8> {
        borsh::to_vec(id).expect("borsh serialization should not fail")
    }

    fn ser_record(record: &EeSerializableTaskRecord) -> Vec<u8> {
        borsh::to_vec(record).expect("borsh serialization should not fail")
    }

    fn deser_record(bytes: &[u8]) -> EeSerializableTaskRecord {
        borsh::from_slice(bytes).expect("borsh deserialization should not fail")
    }

    fn deser_key(bytes: &[u8]) -> EeSerializableTaskId {
        borsh::from_slice(bytes).expect("borsh deserialization should not fail")
    }
}

impl TaskStore<EeProofTask> for SledTaskStore {
    fn get_uuid(&self, task_id: &TaskId<EeProofTask>) -> Option<String> {
        let key = Self::ser_key(&to_sled_task_id(task_id));
        self.task_tree
            .get(key)
            .ok()?
            .map(|v| Self::deser_record(&v).uuid)
    }

    fn get_task(
        &self,
        task_id: &TaskId<EeProofTask>,
    ) -> Option<TaskRecord<TaskId<EeProofTask>>> {
        let key = Self::ser_key(&to_sled_task_id(task_id));
        let bytes = self.task_tree.get(key).ok()??;
        let rec = Self::deser_record(&bytes);
        Some(TaskRecord::new(
            from_sled_task_id(&rec.task_id),
            rec.uuid,
            rec.status,
        ))
    }

    fn get_task_by_uuid(&self, uuid: &str) -> Option<TaskRecord<TaskId<EeProofTask>>> {
        let task_id_bytes = self.uuid_tree.get(uuid.as_bytes()).ok()??;
        let sled_task_id = Self::deser_key(&task_id_bytes);
        let key = Self::ser_key(&sled_task_id);
        let bytes = self.task_tree.get(key).ok()??;
        let rec = Self::deser_record(&bytes);
        Some(TaskRecord::new(
            from_sled_task_id(&rec.task_id),
            rec.uuid,
            rec.status,
        ))
    }

    fn insert_task(
        &self,
        record: TaskRecord<TaskId<EeProofTask>>,
    ) -> ProverServiceResult<()> {
        let sled_id = to_sled_task_id(record.task_id());
        let key = Self::ser_key(&sled_id);

        if self.task_tree.contains_key(&key).unwrap_or(false) {
            return Err(ProverServiceError::Config(format!(
                "task already exists: {:?}",
                record.task_id()
            )));
        }

        let sled_rec = EeSerializableTaskRecord {
            task_id: sled_id,
            uuid: record.uuid().to_string(),
            status: record.status().clone(),
            created_at_secs: now_secs(),
            updated_at_secs: now_secs(),
        };

        self.task_tree
            .insert(&key, Self::ser_record(&sled_rec))
            .map_err(|e| ProverServiceError::Internal(anyhow::anyhow!("sled insert: {e}")))?;
        self.uuid_tree
            .insert(record.uuid().as_bytes(), key)
            .map_err(|e| ProverServiceError::Internal(anyhow::anyhow!("sled uuid insert: {e}")))?;
        Ok(())
    }

    fn update_status(
        &self,
        task_id: &TaskId<EeProofTask>,
        status: TaskStatus,
    ) -> ProverServiceResult<()> {
        let sled_id = to_sled_task_id(task_id);
        let key = Self::ser_key(&sled_id);

        let bytes = self
            .task_tree
            .get(&key)
            .map_err(|e| ProverServiceError::Internal(anyhow::anyhow!("sled get: {e}")))?
            .ok_or_else(|| {
                ProverServiceError::TaskNotFound(format!("task not found: {task_id:?}"))
            })?;

        let mut rec = Self::deser_record(&bytes);
        rec.status = status;
        rec.updated_at_secs = now_secs();

        self.task_tree
            .insert(&key, Self::ser_record(&rec))
            .map_err(|e| ProverServiceError::Internal(anyhow::anyhow!("sled update: {e}")))?;
        Ok(())
    }

    fn list_tasks(
        &self,
        filter: Box<dyn Fn(&TaskStatus) -> bool + '_>,
    ) -> Vec<TaskRecord<TaskId<EeProofTask>>> {
        self.task_tree
            .iter()
            .filter_map(|r| r.ok())
            .map(|(_k, v)| Self::deser_record(&v))
            .filter(|rec| filter(&rec.status))
            .map(|rec| TaskRecord::new(from_sled_task_id(&rec.task_id), rec.uuid, rec.status))
            .collect()
    }

    fn count(&self) -> usize {
        self.task_tree.len()
    }
}
