//! reusable utils.

use strata_ol_chain_types::verify_sequencer_signature;
use strata_primitives::params::Params;
use strata_state::batch::SignedCheckpoint;

/// Verify checkpoint has correct signature from sequencer.
pub fn verify_checkpoint_sig(signed_checkpoint: &SignedCheckpoint, params: &Params) -> bool {
    let msg = signed_checkpoint.checkpoint().hash();
    let sig = signed_checkpoint.signature();
    verify_sequencer_signature(params.rollup(), &msg, sig)
}
