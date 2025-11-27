use strata_db_types::{DbResult, errors::DbError, traits::ProofDatabase};
use strata_primitives::proof::{ProofContext, ProofKey};
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
    ) -> Result<Option<SerializableTaskRecord>, typed_sled::error::Error> {
        self.paas_task_tree.get(task_id)
    }

    /// Get TaskId by UUID
    pub fn get_task_id_by_uuid(
        &self,
        uuid: &str,
    ) -> Result<Option<SerializableTaskId>, typed_sled::error::Error> {
        self.paas_uuid_index_tree.get(&uuid.to_string())
    }

    /// Insert a task record (both task tree and UUID index)
    pub fn insert_task(
        &self,
        task_id: &SerializableTaskId,
        record: &SerializableTaskRecord,
    ) -> Result<(), typed_sled::error::Error> {
        self.paas_task_tree.insert(task_id, record)?;
        self.paas_uuid_index_tree.insert(&record.uuid, task_id)?;
        Ok(())
    }

    /// Update task record
    pub fn update_task(
        &self,
        task_id: &SerializableTaskId,
        record: &SerializableTaskRecord,
    ) -> Result<(), typed_sled::error::Error> {
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

    use super::*;
    use crate::sled_db_test_setup;

    sled_db_test_setup!(ProofDBSled, proof_db_tests);
}
