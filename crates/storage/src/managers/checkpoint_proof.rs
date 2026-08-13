//! High-level manager for the checkpoint-proof database.
//!
//! Checkpoint proofs are keyed by the [`EpochCommitment`] they attest to.
//! Other proof kinds (EE chunk, EE acct, ...) will have their own peer
//! managers, each with a domain-specific key type — no shared enum, no
//! opaque-byte scheme.

use std::sync::Arc;

use strata_db_types::checkpoint_proof::{CheckpointProofDatabase, ProofReceiptEntry};
use strata_db_types::{DbError, DbResult};
use strata_identifiers::EpochCommitment;
use tokio::runtime::Handle;
use zkaleido::ProofReceiptWithMetadata;

use crate::ops::checkpoint_proof::CheckpointProofDbOps;

/// Encodes a receipt into the opaque payload the database stores.
fn encode_receipt(receipt: &ProofReceiptWithMetadata) -> ProofReceiptEntry {
    ProofReceiptEntry::new(receipt.encode())
}

/// Decodes an opaque database payload back into a receipt.
fn decode_receipt(entry: &ProofReceiptEntry) -> DbResult<ProofReceiptWithMetadata> {
    ProofReceiptWithMetadata::decode(entry.as_bytes())
        .map_err(|err| DbError::CodecError(format!("proof receipt (inner error: {err})")))
}

#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct CheckpointProofDbManager {
    ops: CheckpointProofDbOps,
}

impl CheckpointProofDbManager {
    pub fn new(handle: Handle, db: Arc<impl CheckpointProofDatabase + 'static>) -> Self {
        let ops = CheckpointProofDbOps::new(handle, db);
        Self { ops }
    }

    pub fn put_proof(
        &self,
        epoch: EpochCommitment,
        proof: ProofReceiptWithMetadata,
    ) -> DbResult<()> {
        self.ops.put_proof_blocking(epoch, encode_receipt(&proof))
    }

    pub fn get_proof(&self, epoch: &EpochCommitment) -> DbResult<Option<ProofReceiptWithMetadata>> {
        self.ops
            .get_proof_blocking(*epoch)?
            .as_ref()
            .map(decode_receipt)
            .transpose()
    }

    pub fn del_proof(&self, epoch: EpochCommitment) -> DbResult<bool> {
        self.ops.del_proof_blocking(epoch)
    }
}
