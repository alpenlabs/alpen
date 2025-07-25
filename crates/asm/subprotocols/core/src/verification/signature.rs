//! Signature verification for checkpoint transactions
//!
//! Handles verification of sequencer signatures on checkpoint data.

use strata_primitives::{
    batch::{SignedCheckpoint, verify_signed_checkpoint_sig},
    block_credential::CredRule,
    buf::Buf32,
};

use crate::error::*;

/// Verifies the signature on a signed checkpoint using the sequencer public key
///
/// # Arguments
/// * `signed_checkpoint` - The signed checkpoint to verify
/// * `sequencer_pubkey` - The authorized sequencer's public key
///
/// # Returns
/// Result indicating if the signature is valid
pub(crate) fn verify_checkpoint_signature(
    signed_checkpoint: &SignedCheckpoint,
    sequencer_pubkey: &Buf32,
) -> Result<()> {
    let cred_rule = CredRule::SchnorrKey(*sequencer_pubkey);

    if verify_signed_checkpoint_sig(signed_checkpoint, &cred_rule) {
        Ok(())
    } else {
        Err(CoreError::InvalidSignature)
    }
}
