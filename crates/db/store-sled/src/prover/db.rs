use strata_db_types::{
    DbResult,
    errors::DbError,
    traits::{ProofDatabase, ProverTaskDatabase},
};
use strata_paas::TaskRecordData;
use strata_primitives::proof::{ProofContext, ProofKey};
use zkaleido::ProofReceiptWithMetadata;

use super::schemas::{ProofDepsSchema, ProofSchema, ProverTaskTree};
use crate::define_sled_database;

define_sled_database!(
    pub struct ProofDBSled {
        proof_tree: ProofSchema,
        proof_deps_tree: ProofDepsSchema,
        prover_task_tree: ProverTaskTree,
    }
);

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

impl ProverTaskDatabase for ProofDBSled {
    fn get_task(&self, key: Vec<u8>) -> DbResult<Option<TaskRecordData>> {
        Ok(self.prover_task_tree.get(&key)?)
    }

    fn insert_task(&self, key: Vec<u8>, record: TaskRecordData) -> DbResult<()> {
        // Matches the `put_proof` pattern: typed_sled's `compare_and_swap`
        // collapses both CAS failure and IO into a single error, so we do
        // the existence check before writing.
        if self.prover_task_tree.get(&key)?.is_some() {
            return Err(DbError::EntryAlreadyExists);
        }
        self.prover_task_tree
            .compare_and_swap(key, None, Some(record))?;
        Ok(())
    }

    fn put_task(&self, key: Vec<u8>, record: TaskRecordData) -> DbResult<()> {
        let old = self.prover_task_tree.get(&key)?;
        self.prover_task_tree
            .compare_and_swap(key, old, Some(record))?;
        Ok(())
    }

    fn list_retriable(&self, now_secs: u64) -> DbResult<Vec<(Vec<u8>, TaskRecordData)>> {
        let mut out = Vec::new();
        for item in self.prover_task_tree.iter() {
            let (key, record) = item?;
            if record.status().is_retriable()
                && record.retry_after_secs().is_some_and(|t| t <= now_secs)
            {
                out.push((key, record));
            }
        }
        Ok(out)
    }

    fn list_unfinished(&self) -> DbResult<Vec<(Vec<u8>, TaskRecordData)>> {
        let mut out = Vec::new();
        for item in self.prover_task_tree.iter() {
            let (key, record) = item?;
            if record.status().is_unfinished() {
                out.push((key, record));
            }
        }
        Ok(out)
    }

    fn count_tasks(&self) -> DbResult<usize> {
        let mut n = 0;
        for item in self.prover_task_tree.iter() {
            item?;
            n += 1;
        }
        Ok(n)
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
