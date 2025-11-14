//! ProofStore implementation for PaaS
//!
//! This module implements ProofStore, handling proof persistence
//! and checkpoint submission to the CL client.

use std::{future::Future, pin::Pin, sync::Arc};

use strata_db_store_sled::prover::ProofDBSled;
use strata_db_types::traits::ProofDatabase;
use strata_paas::{ProverServiceError, ProverServiceResult, ProofStore};
use strata_primitives::proof::ProofContext;
use zkaleido::ProofReceiptWithMetadata;

use crate::operators::checkpoint::CheckpointOperator;

use super::task::ProofTask;
use super::proof_key_for;

/// Unified proof storage service that handles all proof types
///
/// This service:
/// - Stores proofs in the database
/// - Submits checkpoint proofs to the CL client
/// - Handles all proof types through the registry system
#[derive(Clone)]
pub(crate) struct ProofStoreService {
    db: Arc<ProofDBSled>,
    checkpoint_operator: CheckpointOperator,
}

impl ProofStoreService {
    pub(crate) fn new(db: Arc<ProofDBSled>, checkpoint_operator: CheckpointOperator) -> Self {
        Self {
            db,
            checkpoint_operator,
        }
    }
}

impl ProofStore<ProofTask> for ProofStoreService {
    fn store_proof<'a>(
        &'a self,
        program: &'a ProofTask,
        proof: ProofReceiptWithMetadata,
    ) -> Pin<Box<dyn Future<Output = ProverServiceResult<()>> + Send + 'a>> {
        Box::pin(async move {
            // Extract ProofContext from ProofTask wrapper
            let proof_context = program.0;

            let proof_key = proof_key_for(proof_context);

            // Store proof in database
            self.db
                .put_proof(proof_key, proof)
                .map_err(|e| ProverServiceError::PermanentFailure(e.to_string()))?;

            // If this is a checkpoint proof, submit it to the CL client
            if let ProofContext::Checkpoint(checkpoint_idx) = proof_context {
                self.checkpoint_operator
                    .submit_checkpoint_proof(checkpoint_idx, &proof_key, &self.db)
                    .await
                    .map_err(|e| {
                        tracing::warn!(
                            %checkpoint_idx,
                            "Failed to submit checkpoint proof to CL: {}",
                            e
                        );
                        ProverServiceError::TransientFailure(format!(
                            "Checkpoint proof stored but CL submission failed: {}",
                            e
                        ))
                    })?;
            }

            Ok(())
        })
    }
}
