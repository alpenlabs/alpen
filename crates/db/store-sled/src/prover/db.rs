use strata_db_types::{DbResult, errors::DbError, traits::ProofDatabase};
use strata_primitives::proof::{ProofContext, ProofKey};
use typed_sled::error::Error;
use zkaleido::ProofReceiptWithMetadata;

use super::schemas::{
    PaasTaskTree, PaasUuidIndexTree, ProofDepsSchema, ProofSchema, SerializableTaskId,
    SerializableTaskRecord,
};
use crate::define_sled_database;

define_sled_database!(
    pub struct ProofDBSled {
        proof_tree: ProofSchema,
        proof_deps_tree: ProofDepsSchema,
        // PaaS task tracking trees
        paas_task_tree: PaasTaskTree,
        paas_uuid_index_tree: PaasUuidIndexTree,
    }
);

impl ProofDBSled {
    /// Get task by TaskId
    pub fn get_task(
        &self,
        task_id: &SerializableTaskId,
    ) -> Result<Option<SerializableTaskRecord>, Error> {
        self.paas_task_tree.get(task_id)
    }

    /// Get TaskId by UUID
    pub fn get_task_id_by_uuid(&self, uuid: &str) -> Result<Option<SerializableTaskId>, Error> {
        self.paas_uuid_index_tree.get(&uuid.to_string())
    }

    /// Insert a task record (both task tree and UUID index)
    pub fn insert_task(
        &self,
        task_id: &SerializableTaskId,
        record: &SerializableTaskRecord,
    ) -> Result<(), Error> {
        self.paas_task_tree.insert(task_id, record)?;
        self.paas_uuid_index_tree.insert(&record.uuid, task_id)?;
        Ok(())
    }

    /// Update task record
    pub fn update_task(
        &self,
        task_id: &SerializableTaskId,
        record: &SerializableTaskRecord,
    ) -> Result<(), Error> {
        self.paas_task_tree.insert(task_id, record)?;
        Ok(())
    }

    /// List all tasks (helper to avoid private iterator types)
    pub fn list_all_tasks(&self) -> Vec<(SerializableTaskId, SerializableTaskRecord)> {
        self.paas_task_tree
            .iter()
            .filter_map(|result| result.ok())
            .collect()
    }
}

impl ProofDatabase for ProofDBSled {
    fn put_proof(&self, proof_key: ProofKey, proof: ProofReceiptWithMetadata) -> DbResult<()> {
        if self.proof_tree.get(&proof_key)?.is_some() {
            return Err(DbError::EntryAlreadyExists);
        }

        self.proof_tree
            .compare_and_swap(proof_key, None, Some(proof))?;
        Ok(())
    }

    fn get_proof(&self, proof_key: &ProofKey) -> DbResult<Option<ProofReceiptWithMetadata>> {
        Ok(self.proof_tree.get(proof_key)?)
    }

    fn del_proof(&self, proof_key: ProofKey) -> DbResult<bool> {
        let old = self.proof_tree.get(&proof_key)?;
        let existed = old.is_some();
        self.proof_tree.compare_and_swap(proof_key, old, None)?;
        Ok(existed)
    }

    fn put_proof_deps(&self, proof_context: ProofContext, deps: Vec<ProofContext>) -> DbResult<()> {
        let old = self.proof_deps_tree.get(&proof_context)?;
        if old.is_some() {
            return Err(DbError::EntryAlreadyExists);
        }

        self.proof_deps_tree
            .compare_and_swap(proof_context, old, Some(deps))?;
        Ok(())
    }

    fn get_proof_deps(&self, proof_context: ProofContext) -> DbResult<Option<Vec<ProofContext>>> {
        Ok(self.proof_deps_tree.get(&proof_context)?)
    }

    fn del_proof_deps(&self, proof_context: ProofContext) -> DbResult<bool> {
        let old = self.proof_deps_tree.get(&proof_context)?;
        let existed = old.is_some();
        self.proof_deps_tree
            .compare_and_swap(proof_context, old, None)?;
        Ok(existed)
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::proof_db_tests;
    use strata_paas::TaskStatus;
    use strata_primitives::proof::{ProofContext, ProofZkVm};
    use zkaleido::{Proof, ProofMetadata, ProofReceipt, PublicValues, ZkVm};

    use super::*;
    use crate::sled_db_test_setup;

    sled_db_test_setup!(ProofDBSled, proof_db_tests);

    fn test_proof_receipt() -> ProofReceiptWithMetadata {
        let proof = Proof::default();
        let public_values = PublicValues::default();
        let receipt = ProofReceipt::new(proof, public_values);
        let metadata = ProofMetadata::new(ZkVm::Native, "0.1".to_string());
        ProofReceiptWithMetadata::new(receipt, metadata)
    }

    #[test]
    fn test_proof_tree_ordering_over_256() {
        let db = setup_db();
        let max_height = 300u64;

        for height in 0..=max_height {
            let context = ProofContext::Checkpoint(height);
            let key = ProofKey::new(context, ProofZkVm::Native);
            db.put_proof(key, test_proof_receipt())
                .expect("insert proof");
        }

        let entries: Vec<(ProofKey, ProofReceiptWithMetadata)> =
            db.proof_tree.iter().filter_map(|res| res.ok()).collect();
        assert_eq!(entries.len(), (max_height + 1) as usize);

        let last_key = entries.last().unwrap().0;
        match last_key.context() {
            ProofContext::Checkpoint(height) => assert_eq!(*height, max_height),
            _ => panic!("unexpected proof context"),
        }
    }

    #[test]
    fn test_proof_deps_ordering_over_256() {
        let db = setup_db();
        let max_height = 300u64;

        for height in 0..=max_height {
            let context = ProofContext::Checkpoint(height);
            db.put_proof_deps(context, vec![]).expect("insert deps");
        }

        let entries: Vec<(ProofContext, Vec<ProofContext>)> = db
            .proof_deps_tree
            .iter()
            .filter_map(|res| res.ok())
            .collect();
        assert_eq!(entries.len(), (max_height + 1) as usize);

        let last_context = entries.last().unwrap().0;
        match last_context {
            ProofContext::Checkpoint(height) => assert_eq!(height, max_height),
            _ => panic!("unexpected proof context"),
        }
    }

    #[test]
    fn test_paas_task_ordering_over_256() {
        let db = setup_db();
        let max_height = 300u64;

        for height in 0..=max_height {
            let task_id = SerializableTaskId {
                program: ProofContext::Checkpoint(height),
                backend: 0,
            };
            let record = SerializableTaskRecord {
                task_id: task_id.clone(),
                uuid: format!("task-{height}"),
                status: TaskStatus::Pending,
                created_at_secs: height,
                updated_at_secs: height,
            };
            db.insert_task(&task_id, &record).expect("insert task");
        }

        let tasks = db.list_all_tasks();
        assert_eq!(tasks.len(), (max_height + 1) as usize);

        let last_task_id = &tasks.last().unwrap().0;
        match last_task_id.program {
            ProofContext::Checkpoint(height) => assert_eq!(height, max_height),
            _ => panic!("unexpected task context"),
        }
    }
}
