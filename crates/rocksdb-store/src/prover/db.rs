use std::sync::Arc;

use rockbound::{OptimisticTransactionDB, SchemaDBOperationsExt, TransactionRetry};
use strata_db::{errors::DbError, traits::ProofDatabase, DbResult};
use strata_primitives::proof::{ProofContext, ProofKey};
use zkaleido::ProofReceipt;

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
    fn put_proof(&self, proof_key: ProofKey, proof: ProofReceipt) -> DbResult<()> {
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

    fn get_proof(&self, proof_key: &ProofKey) -> DbResult<Option<ProofReceipt>> {
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
    use strata_primitives::{
        buf::Buf32,
        evm_exec::EvmEeBlockCommitment,
        l1::L1BlockCommitment,
        l2::L2BlockCommitment,
        proof::{ProofContext, ProofZkVm},
    };
    use zkaleido::{Proof, PublicValues};

    use super::*;
    use crate::test_utils::get_rocksdb_tmp_instance_for_prover;

    fn setup_db() -> ProofDb {
        let (db, db_ops) = get_rocksdb_tmp_instance_for_prover().unwrap();
        ProofDb::new(db, db_ops)
    }

    fn generate_proof() -> (ProofKey, ProofReceipt) {
        let proof_context = ProofContext::BtcBlockspace(
            0,
            L1BlockCommitment::default(),
            L1BlockCommitment::default(),
        );
        let host = ProofZkVm::Native;
        let proof_key = ProofKey::new(proof_context, host);
        let proof = Proof::default();
        let public_values = PublicValues::default();
        let proof_receipt = ProofReceipt::new(proof, public_values);
        (proof_key, proof_receipt)
    }

    fn generate_proof_context_with_deps() -> (ProofContext, Vec<ProofContext>) {
        // Constants for test block IDs
        const BLOCK_1_ID: [u8; 32] = [1u8; 32];
        const BLOCK_2_ID: [u8; 32] = [2u8; 32];

        // Create block IDs
        let evm_block_1 = Buf32::from(BLOCK_1_ID);
        let evm_block_2 = Buf32::from(BLOCK_2_ID);

        // Create L2 block commitments
        let evm_commitment_1 = EvmEeBlockCommitment::new(1, evm_block_1);
        let evm_commitment_2 = EvmEeBlockCommitment::new(2, evm_block_2);

        // Create main proof context
        let main_context =
            ProofContext::ClStf(L2BlockCommitment::null(), L2BlockCommitment::null());

        // Create dependency proof contexts
        let deps = vec![ProofContext::EvmEeStf(evm_commitment_1, evm_commitment_2)];

        (main_context, deps)
    }

    #[test]
    fn test_insert_new_proof() {
        let db = setup_db();

        let (proof_key, proof) = generate_proof();

        let result = db.put_proof(proof_key, proof.clone());
        assert!(
            result.is_ok(),
            "ProofReceipt should be inserted successfully"
        );

        let stored_proof = db.get_proof(&proof_key).unwrap();
        assert_eq!(stored_proof, Some(proof));
    }

    #[test]
    fn test_insert_duplicate_proof() {
        let db = setup_db();

        let (proof_key, proof) = generate_proof();

        db.put_proof(proof_key, proof.clone()).unwrap();

        let result = db.put_proof(proof_key, proof);
        assert!(result.is_err(), "Duplicate proof insertion should fail");
    }

    #[test]
    fn test_get_nonexistent_proof() {
        let db = setup_db();

        let (proof_key, proof) = generate_proof();
        db.put_proof(proof_key, proof.clone()).unwrap();

        let res = db.del_proof(proof_key);
        assert!(matches!(res, Ok(true)));

        let res = db.del_proof(proof_key);
        assert!(matches!(res, Ok(false)));

        let stored_proof = db.get_proof(&proof_key).unwrap();
        assert_eq!(stored_proof, None, "Nonexistent proof should return None");
    }

    #[test]
    fn test_insert_new_deps() {
        let db = setup_db();

        let (proof_context, deps) = generate_proof_context_with_deps();

        let result = db.put_proof_deps(proof_context, deps.clone());
        assert!(
            result.is_ok(),
            "ProofReceipt should be inserted successfully"
        );

        let stored_deps = db.get_proof_deps(proof_context).unwrap();
        assert_eq!(stored_deps, Some(deps));
    }

    #[test]
    fn test_insert_duplicate_proof_deps() {
        let db = setup_db();

        let (proof_context, deps) = generate_proof_context_with_deps();

        db.put_proof_deps(proof_context, deps.clone()).unwrap();

        let result = db.put_proof_deps(proof_context, deps);
        assert!(
            result.is_err(),
            "Duplicate proof deps insertion should fail"
        );
    }

    #[test]
    fn test_get_nonexistent_proof_deps() {
        let db = setup_db();

        let (proof_context, deps) = generate_proof_context_with_deps();
        db.put_proof_deps(proof_context, deps.clone()).unwrap();

        let res = db.del_proof_deps(proof_context);
        assert!(matches!(res, Ok(true)));

        let res = db.del_proof_deps(proof_context);
        assert!(matches!(res, Ok(false)));

        let stored_proof = db.get_proof_deps(proof_context).unwrap();
        assert_eq!(
            stored_proof, None,
            "Nonexistent proof deps should return None"
        );
    }
}
