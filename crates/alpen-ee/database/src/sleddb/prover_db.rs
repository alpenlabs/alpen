//! Sled-backed persistence for EE prover state.
//!
//! Three concerns, all backed by trees on the alpen-client sled
//! instance (separate from OL's):
//! - **Shared prover task store** (`prover_task_tree`) — chunk and acct provers share one physical
//!   tree; key-prefixing on the caller side disambiguates.
//! - **Chunk proof receipts** (`chunk_receipt_tree`) — keyed by task bytes; the acct `fetch_input`
//!   reads these to assemble chunk inputs.
//! - **Acct proof receipts** (`acct_proof_tree` + `acct_proof_id_index_tree`) — keyed by
//!   [`BatchId`], with a secondary index from `ProofId` so `BatchProver::get_proof(proof_id)` is an
//!   O(1) lookup.
//!
//! This crate provides the low-level DB; the paas-facing managers
//! (`TaskStore` / `ReceiptStore` impls, typed `BatchProof` API) live
//! in `bin/alpen-client/src/prover/storage.rs`.

use std::sync::Arc;

use alpen_ee_common::{BatchId, ProofId};
use strata_db_store_sled::SledDbConfig;
use strata_db_types::{errors::DbError, traits::ProverTaskDatabase, DbResult};
use strata_paas::TaskRecordData;
use typed_sled::{SledDb, SledTree};
use zkaleido::ProofReceiptWithMetadata;

use super::{
    AcctProofIdIndexSchema, AcctProofReceiptSchema, ChunkProofReceiptSchema, ProverTaskSchema,
};
use crate::serialization_types::DBBatchId;

/// Combined sled database for all prover-side persistence.
///
/// One struct so `EeDatabases` only carries a single handle; the
/// managers in `bin/alpen-client` hold `Arc<EeProverDbSled>` and
/// project the relevant trees.
#[derive(Debug)]
pub struct EeProverDbSled {
    prover_task_tree: SledTree<ProverTaskSchema>,
    chunk_receipt_tree: SledTree<ChunkProofReceiptSchema>,
    acct_proof_tree: SledTree<AcctProofReceiptSchema>,
    acct_proof_id_index_tree: SledTree<AcctProofIdIndexSchema>,
    #[expect(
        dead_code,
        reason = "kept for parity with other sled DBs; config-driven retries TBD"
    )]
    config: SledDbConfig,
}

impl EeProverDbSled {
    pub fn new(db: Arc<SledDb>, config: SledDbConfig) -> DbResult<Self> {
        Ok(Self {
            prover_task_tree: db.get_tree()?,
            chunk_receipt_tree: db.get_tree()?,
            acct_proof_tree: db.get_tree()?,
            acct_proof_id_index_tree: db.get_tree()?,
            config,
        })
    }

    // ---- Chunk receipt store (paas::ReceiptStore shape) ----

    pub fn put_chunk_receipt(
        &self,
        key: Vec<u8>,
        receipt: ProofReceiptWithMetadata,
    ) -> DbResult<()> {
        self.chunk_receipt_tree.insert(&key, &receipt)?;
        Ok(())
    }

    pub fn get_chunk_receipt(&self, key: &[u8]) -> DbResult<Option<ProofReceiptWithMetadata>> {
        Ok(self.chunk_receipt_tree.get(&key.to_vec())?)
    }

    /// Removes a chunk receipt, returning `true` if a row existed.
    ///
    /// Admin-only path (offline dbtool). Callers must keep the node down
    /// to avoid racing with chunk-prover writes.
    pub fn delete_chunk_receipt(&self, key: &[u8]) -> DbResult<bool> {
        Ok(self.chunk_receipt_tree.take(&key.to_vec())?.is_some())
    }

    // ---- Acct proof store (typed BatchId API) ----

    pub fn put_acct_proof(
        &self,
        batch_id: BatchId,
        receipt: ProofReceiptWithMetadata,
    ) -> DbResult<()> {
        let db_id: DBBatchId = batch_id.into();
        let proof_id = proof_id_for(batch_id);
        // Upsert is fine: idempotent re-submits from paas replace the
        // receipt. The index entry is idempotent too.
        self.acct_proof_tree.insert(&db_id, &receipt)?;
        self.acct_proof_id_index_tree
            .insert(&proof_id, &batch_id.into())?;
        Ok(())
    }

    pub fn get_acct_proof(&self, batch_id: BatchId) -> DbResult<Option<ProofReceiptWithMetadata>> {
        let db_id: DBBatchId = batch_id.into();
        Ok(self.acct_proof_tree.get(&db_id)?)
    }

    pub fn has_acct_proof(&self, batch_id: BatchId) -> DbResult<bool> {
        Ok(self.get_acct_proof(batch_id)?.is_some())
    }

    pub fn get_acct_proof_by_id(
        &self,
        proof_id: ProofId,
    ) -> DbResult<Option<ProofReceiptWithMetadata>> {
        let Some(db_id) = self.acct_proof_id_index_tree.get(&proof_id)? else {
            return Ok(None);
        };
        let batch_id: BatchId = db_id.into();
        self.get_acct_proof(batch_id)
    }

    /// Removes an acct proof along with its secondary index entry,
    /// returning `true` if the proof row existed.
    ///
    /// Admin-only path (offline dbtool). The two trees are not deleted
    /// in a single transaction — acceptable because callers stop the
    /// node before invoking this, so no concurrent writer can observe
    /// the intermediate state.
    pub fn delete_acct_proof(&self, batch_id: BatchId) -> DbResult<bool> {
        let db_id: DBBatchId = batch_id.into();
        let proof_id = proof_id_for(batch_id);
        let existed = self.acct_proof_tree.take(&db_id)?.is_some();
        self.acct_proof_id_index_tree.remove(&proof_id)?;
        Ok(existed)
    }
}

impl ProverTaskDatabase for EeProverDbSled {
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

    fn delete_task(&self, key: Vec<u8>) -> DbResult<bool> {
        let old = self.prover_task_tree.get(&key)?;
        let existed = old.is_some();
        self.prover_task_tree.compare_and_swap(key, old, None)?;
        Ok(existed)
    }

    fn list_retriable(&self, now_secs: u64) -> DbResult<Vec<(Vec<u8>, TaskRecordData)>> {
        let mut out = Vec::new();
        for item in self.prover_task_tree.iter() {
            let (key, record) = item?;
            if record.status().wants_rescan()
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

    fn list_all_tasks(&self) -> DbResult<Vec<(Vec<u8>, TaskRecordData)>> {
        let mut out = Vec::new();
        for item in self.prover_task_tree.iter() {
            out.push(item?);
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

/// `ProofId` for a batch — its `last_block` hash.
///
/// Duplicates the mapping from `bin/alpen-client/src/prover/storage.rs`
/// kept in sync by `BatchProver::get_proof(proof_id)` callers. Moving
/// it here so the sled-layer index and the in-memory index agree.
fn proof_id_for(batch_id: BatchId) -> ProofId {
    batch_id.last_block()
}

#[cfg(test)]
mod tests {
    use strata_acct_types::Hash;
    use zkaleido::{ProgramId, Proof, ProofMetadata, ProofReceipt, ProofType, PublicValues, ZkVm};

    use super::*;

    fn setup_db() -> EeProverDbSled {
        let sled_db = sled::Config::new().temporary(true).open().unwrap();
        let typed_sled = Arc::new(SledDb::new(sled_db).unwrap());
        let config = SledDbConfig::new_with_constant_backoff(2, 0);
        EeProverDbSled::new(typed_sled, config).unwrap()
    }

    fn dummy_receipt() -> ProofReceiptWithMetadata {
        let receipt = ProofReceipt::new(Proof::default(), PublicValues::default());
        let metadata = ProofMetadata::new(
            ZkVm::Native,
            ProgramId([0u8; 32]),
            "0.1".to_string(),
            ProofType::Groth16,
        );
        ProofReceiptWithMetadata::new(receipt, metadata)
    }

    fn hash_from_u8(seed: u8) -> Hash {
        let mut bytes = [0u8; 32];
        bytes[0] = 1;
        bytes[31] = seed;
        Hash::from(bytes)
    }

    #[test]
    fn delete_chunk_receipt_roundtrip() {
        let db = setup_db();
        let key = b"chunk-key".to_vec();
        let receipt = dummy_receipt();

        assert!(matches!(db.delete_chunk_receipt(&key), Ok(false)));

        db.put_chunk_receipt(key.clone(), receipt).unwrap();
        assert!(db.get_chunk_receipt(&key).unwrap().is_some());

        assert!(matches!(db.delete_chunk_receipt(&key), Ok(true)));
        assert!(matches!(db.delete_chunk_receipt(&key), Ok(false)));
        assert!(db.get_chunk_receipt(&key).unwrap().is_none());
    }

    #[test]
    fn delete_acct_proof_clears_primary_and_secondary_rows() {
        let db = setup_db();
        let batch_id = BatchId::from_parts(hash_from_u8(1), hash_from_u8(2));
        let proof_id: ProofId = batch_id.last_block();

        // Missing primary row reports false; secondary index already absent
        // so the call is a clean no-op.
        assert!(matches!(db.delete_acct_proof(batch_id), Ok(false)));

        db.put_acct_proof(batch_id, dummy_receipt()).unwrap();
        assert!(db.has_acct_proof(batch_id).unwrap());
        assert!(db.get_acct_proof_by_id(proof_id).unwrap().is_some());

        assert!(matches!(db.delete_acct_proof(batch_id), Ok(true)));
        assert!(!db.has_acct_proof(batch_id).unwrap());
        // Secondary index entry must be cleared so the by-id lookup also
        // misses — otherwise the index would dangle.
        assert!(db.get_acct_proof_by_id(proof_id).unwrap().is_none());

        // Second delete is idempotent.
        assert!(matches!(db.delete_acct_proof(batch_id), Ok(false)));
    }
}
