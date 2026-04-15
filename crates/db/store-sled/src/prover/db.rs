use strata_db_types::{
    DbResult,
    errors::DbError,
    traits::{CheckpointProofDatabase, ProverTaskDatabase},
};
use strata_identifiers::EpochCommitment;
use strata_paas::TaskRecordData;
use zkaleido::ProofReceiptWithMetadata;

use super::schemas::{CheckpointProofSchema, ProverTaskTree};
use crate::define_sled_database;

define_sled_database!(
    pub struct ProofDBSled {
        checkpoint_proof_tree: CheckpointProofSchema,
        prover_task_tree: ProverTaskTree,
    }
);

impl CheckpointProofDatabase for ProofDBSled {
    fn put_proof(&self, epoch: EpochCommitment, proof: ProofReceiptWithMetadata) -> DbResult<()> {
        if self.checkpoint_proof_tree.get(&epoch)?.is_some() {
            return Err(DbError::EntryAlreadyExists);
        }
        self.checkpoint_proof_tree
            .compare_and_swap(epoch, None, Some(proof))?;
        Ok(())
    }

    fn get_proof(&self, epoch: EpochCommitment) -> DbResult<Option<ProofReceiptWithMetadata>> {
        Ok(self.checkpoint_proof_tree.get(&epoch)?)
    }

    fn del_proof(&self, epoch: EpochCommitment) -> DbResult<bool> {
        let old = self.checkpoint_proof_tree.get(&epoch)?;
        let existed = old.is_some();
        self.checkpoint_proof_tree
            .compare_and_swap(epoch, old, None)?;
        Ok(existed)
    }
}

impl ProverTaskDatabase for ProofDBSled {
    fn get_task(&self, key: Vec<u8>) -> DbResult<Option<TaskRecordData>> {
        Ok(self.prover_task_tree.get(&key)?)
    }

    fn insert_task(&self, key: Vec<u8>, record: TaskRecordData) -> DbResult<()> {
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
