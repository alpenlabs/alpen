use strata_db::{DbResult, errors::DbError, traits::ProofDatabase};
use strata_primitives::proof::{ProofContext, ProofKey};
use zkaleido::ProofReceiptWithMetadata;

use super::schemas::{ProofDepsSchema, ProofSchema};
use crate::define_sled_database;

define_sled_database!(
    pub struct ProofDBSled {
        proof_tree: ProofSchema,
        proof_deps_tree: ProofDepsSchema,
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

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::proof_db_tests;

    use super::*;
    use crate::sled_db_test_setup;

    sled_db_test_setup!(ProofDBSled, proof_db_tests);
}
