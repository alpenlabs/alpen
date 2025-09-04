use strata_asm_common::MsgRelayer;
use strata_asm_proto_administration_txs::actions::{MultisigAction, UpdateAction};
use strata_crypto::multisig::vote::AggregatedVote;
use strata_primitives::roles::ProofType;

use crate::{
    error::AdministrationError,
    state::AdministrationSubprotoState,
    updates::{queued::QueuedUpdate, scheduled::ScheduledUpdate},
};

pub(crate) fn handle_scheduled_updates(
    state: &mut AdministrationSubprotoState,
    _relayer: &mut impl MsgRelayer,
    current_height: u64,
) {
    // Get all the update actions that are ready to be enacted
    let actions_to_enact = state.process_scheduled(current_height);

    for action in actions_to_enact {
        match action.action() {
            UpdateAction::Multisig(update) => {
                state.apply_multisig_update(update.role(), update.config());
            }
            UpdateAction::VerifyingKey(update) => match update.kind() {
                ProofType::Asm => {
                    // Emit Log
                }
                ProofType::OlStf => {
                    // Send a InterprotoMsg to OL Core subprotocol
                }
            },
            UpdateAction::OperatorSet(_update) => {
                // Set an InterProtoMsg to the Bridge Subprotocol;
            }
            UpdateAction::Sequencer(_update) => {
                // Send a InterprotoMsg to the Sequencer subprotocol
            }
        }
    }
}

pub(crate) fn handle_action(
    state: &mut AdministrationSubprotoState,
    action: MultisigAction,
    vote: AggregatedVote,
    current_height: u64,
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
                // If the action is a VerifyingKeyUpdate, queue it to support cancellation
                UpdateAction::VerifyingKey(_) => {
                    let queued_update = QueuedUpdate::try_new(id, update, current_height)?;
                    state.enqueue(queued_update);
                }
                // For all other actions, directly schedule them for execution
                _ => {
                    let scheduled_update = ScheduledUpdate::try_new(id, update, current_height)?;
                    state.schedule(scheduled_update);
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
