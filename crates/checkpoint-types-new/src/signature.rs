//! Signature verification for signed checkpoints.

use strata_crypto::schnorr::verify_schnorr_sig;
use strata_identifiers::CredRule;

use crate::SignedCheckpointPayload;

/// Verifies that a signed checkpoint payload has a proper signature.
pub fn verify_checkpoint_payload_signature(
    signed_checkpoint: &SignedCheckpointPayload,
    cred_rule: &CredRule,
) -> bool {
    let seq_pubkey = match cred_rule {
        CredRule::SchnorrKey(key) => key,
        // In this case we always just assume true.
        CredRule::Unchecked => return true,
    };

    let checkpoint_hash = signed_checkpoint.payload().compute_hash();
    verify_schnorr_sig(signed_checkpoint.signature(), &checkpoint_hash, seq_pubkey)
}
