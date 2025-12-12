//! Inter-protocol message handlers.
//!
//! This module handles messages from other subprotocols (primarily admin)
//! for updating checkpoint configuration such as sequencer keys and predicates.

use strata_asm_common::logging;
use strata_identifiers::{Buf32, CredRule};
use strata_predicate::PredicateKey;

use crate::state::CheckpointState;

/// Apply a sequencer key update from the admin subprotocol.
pub(crate) fn apply_sequencer_key_update(state: &mut CheckpointState, new_key: Buf32) {
    if matches!(state.sequencer_cred(), CredRule::SchnorrKey(existing) if existing == &new_key) {
        logging::debug!("Sequencer key update received, key unchanged");
        return;
    }

    state.update_sequencer_cred(new_key);
    logging::info!(new_key = %new_key, "Updated sequencer public key");
}

/// Apply a checkpoint predicate update from the admin subprotocol.
pub(crate) fn apply_predicate_update(state: &mut CheckpointState, new_predicate: &PredicateKey) {
    let prev_kind = state.checkpoint_predicate().id();
    let next_kind = new_predicate.id();

    if prev_kind == next_kind {
        logging::debug!(kind = %next_kind, "Checkpoint predicate unchanged");
    } else {
        logging::info!(
            previous = %prev_kind,
            next = %next_kind,
            "Switching checkpoint proving system"
        );
    }

    state.update_checkpoint_predicate(new_predicate.clone());
}
