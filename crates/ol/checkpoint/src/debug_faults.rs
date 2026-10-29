//! Checkpoint fault injection for functional tests.
//!
//! Built only with the `debug-utils` feature, which release images leave off.

use std::sync::atomic::{AtomicBool, Ordering};

use strata_identifiers::EpochCommitment;
use tracing::warn;

/// Set while a request to corrupt the next checkpoint proof is pending.
static CORRUPT_NEXT_PROOF: AtomicBool = AtomicBool::new(false);

/// Makes the checkpoint worker corrupt the proof of the next checkpoint it builds.
///
/// L1 still mines that checkpoint, but the ASM rejects it under the active predicate. Functional
/// tests use this to produce a checkpoint the ASM rejects without a checkpoint predicate
/// enactment, which ends the spec this binary runs. A checkpoint built without a proof leaves the
/// request pending.
pub fn corrupt_next_checkpoint_proof() {
    CORRUPT_NEXT_PROOF.store(true, Ordering::Relaxed);
}

/// Returns `proof` with its last byte flipped if a corruption is pending, and clears the request.
pub(crate) fn apply_requested_proof_corruption(
    commitment: EpochCommitment,
    mut proof: Vec<u8>,
) -> Vec<u8> {
    let Some(last) = proof.last_mut() else {
        return proof;
    };
    if CORRUPT_NEXT_PROOF.swap(false, Ordering::Relaxed) {
        *last ^= 0x01;
        warn!(%commitment, epoch = commitment.epoch(), "corrupted checkpoint proof on request");
    }
    proof
}

#[cfg(test)]
mod tests {
    use strata_identifiers::OLBlockId;

    use super::*;

    #[test]
    fn test_corruption_applies_once_to_the_next_non_empty_proof() {
        let commitment = EpochCommitment::new(2, 8, OLBlockId::null());
        let proof = vec![0xaa, 0xbb];

        assert_eq!(
            apply_requested_proof_corruption(commitment, proof.clone()),
            proof
        );

        corrupt_next_checkpoint_proof();
        assert_eq!(
            apply_requested_proof_corruption(commitment, Vec::new()),
            Vec::<u8>::new()
        );
        assert_eq!(
            apply_requested_proof_corruption(commitment, proof.clone()),
            vec![0xaa, 0xba]
        );
        assert_eq!(
            apply_requested_proof_corruption(commitment, proof.clone()),
            proof
        );
    }
}
