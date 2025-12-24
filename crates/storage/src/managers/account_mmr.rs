use std::sync::Arc;

use strata_db_types::{mmr_helpers::MmrAlgorithm, traits::AccountMmrDatabase, DbResult};
use strata_identifiers::AccountId;
use strata_merkle::MerkleProofB32 as MerkleProof;
use threadpool::ThreadPool;

use crate::ops;

/// Manager for account-scoped MMR (Merkle Mountain Range) database operations
///
/// Provides high-level async/blocking APIs for per-account MMR operations including
/// appending leaves, generating proofs, and accessing MMR metadata.
///
/// Each account maintains its own independent MMR tree with separate indexing
/// and metadata.
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct AccountMmrManager {
    ops: ops::account_mmr::AccountMmrDataOps,
}

impl AccountMmrManager {
    pub fn new(pool: ThreadPool, db: Arc<impl AccountMmrDatabase + 'static>) -> Self {
        let ops = ops::account_mmr::Context::new(db).into_ops(pool);
        Self { ops }
    }

    /// Append a new leaf to the MMR for a specific account (async version)
    pub async fn append_leaf(&self, account: AccountId, hash: [u8; 32]) -> DbResult<u64> {
        self.ops.append_leaf_async(account, hash).await
    }

    /// Append a new leaf to the MMR for a specific account (blocking version)
    pub fn append_leaf_blocking(&self, account: AccountId, hash: [u8; 32]) -> DbResult<u64> {
        self.ops.append_leaf_blocking(account, hash)
    }

    /// Generate a Merkle proof for a single leaf position in an account's MMR
    pub fn generate_proof(&self, account: AccountId, index: u64) -> DbResult<MerkleProof> {
        let mmr_size = self.ops.mmr_size_blocking(account)?;
        let num_leaves = self.ops.num_leaves_blocking(account)?;

        MmrAlgorithm::generate_proof(index, mmr_size, num_leaves, |pos| {
            self.ops.get_node_blocking(account, pos)
        })
    }

    /// Generate Merkle proofs for a range of leaf positions in an account's MMR
    pub fn generate_proofs(
        &self,
        account: AccountId,
        start: u64,
        end: u64,
    ) -> DbResult<Vec<MerkleProof>> {
        let mmr_size = self.ops.mmr_size_blocking(account)?;
        let num_leaves = self.ops.num_leaves_blocking(account)?;

        MmrAlgorithm::generate_proofs(start, end, mmr_size, num_leaves, |pos| {
            self.ops.get_node_blocking(account, pos)
        })
    }

    /// Remove and return the last leaf from the MMR for a specific account (async version)
    pub async fn pop_leaf(&self, account: AccountId) -> DbResult<Option<[u8; 32]>> {
        self.ops.pop_leaf_async(account).await
    }

    /// Remove and return the last leaf from the MMR for a specific account (blocking version)
    pub fn pop_leaf_blocking(&self, account: AccountId) -> DbResult<Option<[u8; 32]>> {
        self.ops.pop_leaf_blocking(account)
    }
}
