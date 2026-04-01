//! High-level manager for proof database access.

use std::sync::Arc;

use strata_db_types::{traits::ProofDatabase, DbResult};
use strata_primitives::proof::{ProofContext, ProofKey};
use threadpool::ThreadPool;
use zkaleido::ProofReceiptWithMetadata;

use crate::ops::proof::{Context, ProofDbOps};

#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct ProofDbManager {
    ops: ProofDbOps,
}

impl ProofDbManager {
    pub fn new(pool: ThreadPool, db: Arc<impl ProofDatabase + 'static>) -> Self {
        let ops = Context::new(db).into_ops(pool);
        Self { ops }
    }

    pub fn put_proof(&self, proof_key: ProofKey, proof: ProofReceiptWithMetadata) -> DbResult<()> {
        self.ops.put_proof_blocking(proof_key, proof)
    }

    pub fn get_proof(&self, proof_key: ProofKey) -> DbResult<Option<ProofReceiptWithMetadata>> {
        self.ops.get_proof_by_key_blocking(proof_key)
    }

    pub fn del_proof(&self, proof_key: ProofKey) -> DbResult<bool> {
        self.ops.del_proof_blocking(proof_key)
    }

    pub fn put_proof_deps(
        &self,
        proof_context: ProofContext,
        deps: Vec<ProofContext>,
    ) -> DbResult<()> {
        self.ops.put_proof_deps_blocking(proof_context, deps)
    }

    pub fn get_proof_deps(
        &self,
        proof_context: ProofContext,
    ) -> DbResult<Option<Vec<ProofContext>>> {
        self.ops.get_proof_deps_blocking(proof_context)
    }

    pub fn del_proof_deps(&self, proof_context: ProofContext) -> DbResult<bool> {
        self.ops.del_proof_deps_blocking(proof_context)
    }
}
