use strata_asm_common::MsgRelayer;
use strata_asm_proto_administration_txs::actions::{MultisigAction, UpdateAction};
use strata_crypto::multisig::vote::AggregatedVote;

use crate::{
    error::AdministrationError, state::AdministrationSubprotoState, updates::queued::QueuedUpdate,
};

pub(crate) fn handle_action(
    state: &mut AdministrationSubprotoState,
    action: MultisigAction,
    vote: AggregatedVote,
    current_height: u64,
    _relayer: &mut impl MsgRelayer,
) -> Result<(), AdministrationError> {
    let role = match &action {
        MultisigAction::Update(update) => update.required_role(),
        MultisigAction::Cancel(cancel) => {
            let target_action_id = cancel.target_id();
            let queued = state
                .find_queued(target_action_id)
                .ok_or(AdministrationError::UnknownAction(*target_action_id))?;
            queued.action().required_role()
        }
    };

    let authority = state
        .authority(role)
        .ok_or(AdministrationError::UnknownRole)?;
    authority.validate_action(&action, &vote)?;

    match action {
        MultisigAction::Update(update) => {
            let id = state.next_update_id();
            match update {
                UpdateAction::Sequencer(_) => {
                    // TODO: directly apply it without waiting
                }
                action => {
                    // For all others add it to queue
                    let queued_update = QueuedUpdate::try_new(id, action, current_height)?;
                    state.enqueue(queued_update);
                }
            }
            state.increment_next_update_id();
        }
        MultisigAction::Cancel(cancel) => {
            state.remove_queued(cancel.target_id());
        }
    }

    // Increase the nonce
    let authority = state
        .authority_mut(role)
        .ok_or(AdministrationError::UnknownRole)?;
    authority.increment_seqno();

    Ok(())
}
