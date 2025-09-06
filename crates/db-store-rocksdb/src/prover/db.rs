use std::sync::Arc;

use rockbound::{OptimisticTransactionDB, SchemaDBOperationsExt, TransactionRetry};
use strata_db::{errors::DbError, traits::ProofDatabase, DbResult};
use strata_primitives::proof::{ProofContext, ProofKey};
use zkaleido::ProofReceiptWithMetadata;

use super::schemas::{ProofDepsSchema, ProofSchema};
use crate::DbOpsConfig;

#[derive(Debug, Clone)]
pub struct ProofDb {
    db: Arc<OptimisticTransactionDB>,
    ops: DbOpsConfig,
}

impl ProofDb {
    pub fn new(db: Arc<OptimisticTransactionDB>, ops: DbOpsConfig) -> Self {
        Self { db, ops }
    }
}

impl ProofDatabase for ProofDb {
    fn put_proof(&self, proof_key: ProofKey, proof: ProofReceiptWithMetadata) -> DbResult<()> {
        self.db
            .with_optimistic_txn(TransactionRetry::Count(self.ops.retry_count), |tx| {
                if tx.get::<ProofSchema>(&proof_key)?.is_some() {
                    return Err(DbError::EntryAlreadyExists);
                }

                tx.put::<ProofSchema>(&proof_key, &proof)?;

                Ok(())
            })
            .map_err(|e| DbError::TransactionError(e.to_string()))
    }

    fn get_proof(&self, proof_key: &ProofKey) -> DbResult<Option<ProofReceiptWithMetadata>> {
        Ok(self.db.get::<ProofSchema>(proof_key)?)
    }

    fn del_proof(&self, proof_key: ProofKey) -> DbResult<bool> {
        self.db
            .with_optimistic_txn(TransactionRetry::Count(self.ops.retry_count), |tx| {
                if tx.get::<ProofSchema>(&proof_key)?.is_none() {
                    return Ok(false);
                }
                tx.delete::<ProofSchema>(&proof_key)?;

                Ok::<_, anyhow::Error>(true)
            })
            .map_err(|e| DbError::TransactionError(e.to_string()))
    }

    fn put_proof_deps(&self, proof_context: ProofContext, deps: Vec<ProofContext>) -> DbResult<()> {
        self.db
            .with_optimistic_txn(TransactionRetry::Count(self.ops.retry_count), |tx| {
                if tx.get::<ProofDepsSchema>(&proof_context)?.is_some() {
                    return Err(DbError::EntryAlreadyExists);
                }

                tx.put::<ProofDepsSchema>(&proof_context, &deps)?;

                Ok(())
            })
            .map_err(|e| DbError::TransactionError(e.to_string()))
    }

    fn get_proof_deps(&self, proof_context: ProofContext) -> DbResult<Option<Vec<ProofContext>>> {
        Ok(self.db.get::<ProofDepsSchema>(&proof_context)?)
    }

    fn del_proof_deps(&self, proof_context: ProofContext) -> DbResult<bool> {
        self.db
            .with_optimistic_txn(TransactionRetry::Count(self.ops.retry_count), |tx| {
                if tx.get::<ProofDepsSchema>(&proof_context)?.is_none() {
                    return Ok(false);
                }
                tx.delete::<ProofDepsSchema>(&proof_context)?;

                Ok::<_, anyhow::Error>(true)
            })
            .map_err(|e| DbError::TransactionError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use strata_db_tests::proof_db_tests;

    use super::*;
    use crate::test_utils::get_rocksdb_tmp_instance_for_prover;

    fn setup_db() -> ProofDb {
        let (db, db_ops) = get_rocksdb_tmp_instance_for_prover().unwrap();
        ProofDb::new(db, db_ops)
    }

    proof_db_tests!(setup_db());
}
