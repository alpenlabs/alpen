//! reusable utils.

use strata_checkpoint_types::SignedCheckpoint;
use strata_ol_chain_types::verify_sequencer_signature;
use strata_params::Params;

/// Verify checkpoint has correct signature from sequencer.
pub fn verify_checkpoint_sig(signed_checkpoint: &SignedCheckpoint, params: &Params) -> bool {
    let msg = signed_checkpoint.checkpoint().hash();
    let sig = signed_checkpoint.signature();
    verify_sequencer_signature(params.rollup(), &msg, sig)
}
