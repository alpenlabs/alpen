//! Adapter to integrate ProofOperator with PaaS ProofOperatorTrait

use std::sync::Arc;

use strata_db_store_sled::prover::ProofDBSled;
use strata_paas::manager::ProofOperatorTrait;
use strata_primitives::proof::ProofKey;

use crate::operators::ProofOperator;

/// Adapter for ProofOperator to implement ProofOperatorTrait
#[derive(Clone)]
pub struct ProofOperatorAdapter {
    inner: Arc<ProofOperator>,
}

impl ProofOperatorAdapter {
    pub fn new(operator: ProofOperator) -> Self {
        Self {
            inner: Arc::new(operator),
        }
    }

    pub fn from_arc(operator: Arc<ProofOperator>) -> Self {
        Self { inner: operator }
    }
}

impl ProofOperatorTrait<ProofDBSled> for ProofOperatorAdapter {
    async fn process_proof(
        &self,
        proof_key: ProofKey,
        db: &ProofDBSled,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.inner
            .process_proof(&proof_key, db)
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    }
}
