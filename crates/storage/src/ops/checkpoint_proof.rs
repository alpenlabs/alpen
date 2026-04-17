//! Checkpoint-proof database operation interface.

use strata_db_types::traits::CheckpointProofDatabase;
use strata_identifiers::EpochCommitment;
use zkaleido::ProofReceiptWithMetadata;

use crate::{exec::*, instrumentation::components};

inst_ops_simple! {
    (<D: CheckpointProofDatabase> => CheckpointProofDbOps, component = components::STORAGE_CHECKPOINT_PROOF) {
        put_proof(epoch: EpochCommitment, proof: ProofReceiptWithMetadata) => ();
        get_proof(epoch: EpochCommitment) => Option<ProofReceiptWithMetadata>;
        del_proof(epoch: EpochCommitment) => bool;
    }
}
