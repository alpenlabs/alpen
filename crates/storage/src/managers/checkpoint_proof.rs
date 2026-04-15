//! High-level manager for the checkpoint-proof database.
//!
//! Checkpoint proofs are keyed by the [`EpochCommitment`] they attest to.
//! Other proof kinds (EE chunk, EE acct, ...) will have their own peer
//! managers, each with a domain-specific key type — no shared enum, no
//! opaque-byte scheme.

use std::sync::Arc;

use strata_db_types::{traits::CheckpointProofDatabase, DbResult};
use strata_identifiers::EpochCommitment;
use threadpool::ThreadPool;
use zkaleido::ProofReceiptWithMetadata;

use crate::ops::checkpoint_proof::{CheckpointProofDbOps, Context};

#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct CheckpointProofDbManager {
    ops: CheckpointProofDbOps,
}

impl CheckpointProofDbManager {
    pub fn new(pool: ThreadPool, db: Arc<impl CheckpointProofDatabase + 'static>) -> Self {
        let ops = Context::new(db).into_ops(pool);
        Self { ops }
    }

    pub fn put_proof(
        &self,
        epoch: EpochCommitment,
        proof: ProofReceiptWithMetadata,
    ) -> DbResult<()> {
        self.ops.put_proof_blocking(epoch, proof)
    }

    pub fn get_proof(&self, epoch: &EpochCommitment) -> DbResult<Option<ProofReceiptWithMetadata>> {
        self.ops.get_proof_blocking(*epoch)
    }

    pub fn del_proof(&self, epoch: EpochCommitment) -> DbResult<bool> {
        self.ops.del_proof_blocking(epoch)
    }
}
