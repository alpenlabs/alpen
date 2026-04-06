//! Proof storage adapter for checkpoint proofs.
//!
//! Stores generated proof receipts using node storage proof manager and
//! notifies the checkpoint worker that a new proof is available.

use std::sync::Arc;

use async_trait::async_trait;
use strata_ol_checkpoint::ProofNotify;
use strata_paas::ProofStorer;
use strata_primitives::proof::{ProofContext, ProofKey};
use strata_storage::ProofDbManager;
use tracing::info;
use zkaleido::ProofReceiptWithMetadata;

use super::{errors::ProofStorageError, task::CheckpointTask};

/// Stores proof receipts via the shared proof manager.
#[derive(Clone)]
pub(crate) struct CheckpointProofStorer {
    db: Arc<ProofDbManager>,
    proof_notify: Arc<ProofNotify>,
}

impl CheckpointProofStorer {
    pub(crate) fn new(db: Arc<ProofDbManager>, proof_notify: Arc<ProofNotify>) -> Self {
        Self { db, proof_notify }
    }
}

#[async_trait]
impl ProofStorer<CheckpointTask> for CheckpointProofStorer {
    type Error = ProofStorageError;

    async fn store_proof(
        &self,
        program: &CheckpointTask,
        proof: ProofReceiptWithMetadata,
    ) -> Result<(), Self::Error> {
        let zkvm = program
            .proof_zkvm()
            .map_err(|e| ProofStorageError(anyhow::anyhow!(e)))?;
        let epoch = program.commitment.epoch;
        let proof_key = ProofKey::new(ProofContext::CheckpointCommitment(program.commitment), zkvm);
        info!(%epoch, "storing checkpoint proof");
        self.db
            .put_proof(proof_key, proof)
            .map_err(|e| ProofStorageError(e.into()))?;

        // Wake the checkpoint worker so it picks up the proof immediately.
        self.proof_notify.notify();

        Ok(())
    }
}
