//! Receipt hook that writes checkpoint proofs into the proof DB and wakes
//! the checkpoint worker.
//!
//! After the checkpoint prover produces a receipt, this hook:
//! 1. Writes the proof into the [`CheckpointProofDbManager`]
//! 2. Signals [`ProofNotify`] so the checkpoint worker picks up the proof immediately (no polling).

use std::sync::Arc;

use async_trait::async_trait;
use strata_ol_checkpoint::ProofNotify;
use strata_paas::{ProverError, ProverResult, ReceiptHook};
use strata_storage::CheckpointProofDbManager;
use tracing::info;
use zkaleido::ProofReceiptWithMetadata;

use super::spec::{CheckpointSpec, CheckpointTask};

/// [`ReceiptHook`] that persists checkpoint proofs and wakes the checkpoint
/// worker.
pub(crate) struct CheckpointReceiptHook {
    proof_db: Arc<CheckpointProofDbManager>,
    proof_notify: Arc<ProofNotify>,
}

impl CheckpointReceiptHook {
    pub(crate) fn new(
        proof_db: Arc<CheckpointProofDbManager>,
        proof_notify: Arc<ProofNotify>,
    ) -> Self {
        Self {
            proof_db,
            proof_notify,
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
        info!(%epoch, "storing checkpoint proof");
        self.proof_db
            .put_proof(task.0, receipt.clone())
            .map_err(|e| ProverError::Storage(format!("put checkpoint proof: {e}")))?;

        // Wake the checkpoint worker so it picks up the proof immediately.
        self.proof_notify.notify();
        Ok(())
    }
}
