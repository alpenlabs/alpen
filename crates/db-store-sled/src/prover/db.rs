use std::sync::Arc;

use strata_db::{DbResult, errors::DbError, traits::ProofDatabase};
use strata_primitives::proof::{ProofContext, ProofKey};
use typed_sled::{SledDb, SledTree};
use zkaleido::ProofReceiptWithMetadata;

use super::schemas::{ProofDepsSchema, ProofSchema};
use crate::SledDbConfig;

#[derive(Debug)]
pub struct ProofDBSled {
    proof_tree: SledTree<ProofSchema>,
    proof_deps_tree: SledTree<ProofDepsSchema>,
    _config: SledDbConfig,
}

impl ProofDBSled {
    pub fn new(db: Arc<SledDb>, config: SledDbConfig) -> DbResult<Self> {
        Ok(Self {
            proof_tree: db.get_tree()?,
            proof_deps_tree: db.get_tree()?,
            _config: config,
        })
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

    fn setup_db() -> ProofDBSled {
        let db = sled::Config::new().temporary(true).open().unwrap();
        let sled_db = SledDb::new(db).unwrap();
        let config = SledDbConfig::new_with_constant_backoff(3, 200);
        ProofDBSled::new(sled_db.into(), config).unwrap()
    }

    proof_db_tests!(setup_db());
}
