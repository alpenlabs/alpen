//! Receipt hook that writes checkpoint proofs into the proof DB and wakes
//! the checkpoint worker.
//!
//! After the prover produces a receipt, this hook:
//! 1. Writes the proof into the shared [`ProofDbManager`] keyed by
//!    [`ProofContext::CheckpointCommitment`].
//! 2. Signals [`ProofNotify`] so the checkpoint worker picks up the proof
//!    immediately (no polling).

use std::sync::Arc;

use async_trait::async_trait;
use strata_ol_checkpoint::ProofNotify;
use strata_paas::{ProverError, ProverResult, ReceiptHook};
use strata_primitives::proof::{ProofContext, ProofKey, ProofZkVm};
use strata_storage::ProofDbManager;
use tracing::info;
use zkaleido::ProofReceiptWithMetadata;

use super::spec::{CheckpointSpec, CheckpointTask};

/// [`ReceiptHook`] that persists checkpoint proofs and wakes the checkpoint
/// worker.
pub(crate) struct CheckpointReceiptHook {
    proof_db: Arc<ProofDbManager>,
    proof_notify: Arc<ProofNotify>,
    zkvm: ProofZkVm,
}

impl CheckpointReceiptHook {
    pub(crate) fn new(
        proof_db: Arc<ProofDbManager>,
        proof_notify: Arc<ProofNotify>,
        zkvm: ProofZkVm,
    ) -> Self {
        Self {
            proof_db,
            proof_notify,
            zkvm,
        }
    }
}

#[async_trait]
impl ReceiptHook<CheckpointSpec> for CheckpointReceiptHook {
    async fn on_receipt(
        &self,
        task: &CheckpointTask,
        receipt: &ProofReceiptWithMetadata,
    ) -> ProverResult<()> {
        let epoch = task.0.epoch;
        let proof_key = ProofKey::new(ProofContext::CheckpointCommitment(task.0), self.zkvm);
        info!(%epoch, "storing checkpoint proof");
        self.proof_db
            .put_proof(proof_key, receipt.clone())
            .map_err(|e| ProverError::Storage(format!("put checkpoint proof: {e}")))?;

        // Wake the checkpoint worker so it picks up the proof immediately.
        self.proof_notify.notify();
        Ok(())
    }
}
