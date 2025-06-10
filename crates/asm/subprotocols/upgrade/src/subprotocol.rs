use strata_asm_common::{AnchorState, MsgRelayer, NullMsg, Subprotocol, SubprotocolId, TxInput};

use crate::{
    error::UpgradeError,
    multisig::vote::AggregatedVote,
    roles::ProofType,
    state::UpgradeSubprotoState,
    txs::{MultisigAction, UpgradeAction, parse_tx_multisig_action_and_vote},
    upgrades::{queued::QueuedUpgrade, scheduled::ScheduledUpgrade},
};

pub const UPGRADE_SUBPROTOCOL_ID: u8 = 0;

#[derive(Debug)]
pub struct UpgradeSubprotocol;

impl Subprotocol for UpgradeSubprotocol {
    const ID: SubprotocolId = 0;

    type State = UpgradeSubprotoState;

    type Msg = NullMsg<0>;

    fn init() -> UpgradeSubprotoState {
        UpgradeSubprotoState::default()
    }

    fn process_txs(
        state: &mut UpgradeSubprotoState,
        txs: &[TxInput<'_>],
        relayer: &mut impl MsgRelayer,
        anchor_pre: &AnchorState,
    ) {
        // Get the current height
        let current_height = anchor_pre.chain_view.pow_state.last_verified_block.height() + 1;

        // Before processing the transactions, we process any queued actions
        state.process_queued(current_height);

        for tx in txs {
            if let Ok((action, vote)) = parse_tx_multisig_action_and_vote(tx) {
                let _ = handle_action(state, action, vote, current_height);
            }
        }

        handle_scheduled_actions(state, relayer, current_height);
    }

    fn process_msgs(_state: &mut UpgradeSubprotoState, _msgs: &[Self::Msg]) {}
}

fn handle_scheduled_actions(
    state: &mut UpgradeSubprotoState,
    _relayer: &mut impl MsgRelayer,
    current_height: u64,
) {
    // Get all the update actions that are ready to be enacted
    let actions_to_enact = state.process_scheduled(current_height);

    for action in actions_to_enact {
        match action.action() {
            UpgradeAction::Multisig(update) => {
                state.apply_multisig_update(update.role(), update.config());
            }
            UpgradeAction::VerifyingKey(update) => match update.kind() {
                ProofType::Asm => {
                    // Emit Log
                }
                ProofType::OlStf => {
                    // Send a InterprotoMsg to OL Core subprotocol
                }
            },
            UpgradeAction::OperatorSet(_update) => {
                // Set an InterProtoMsg to the Bridge Subprotocol;
            }
            UpgradeAction::Sequencer(_update) => {
                // Send a InterprotoMsg to the Sequencer subprotocol
            }
        }
    }
}

fn handle_action(
    state: &mut UpgradeSubprotoState,
    action: MultisigAction,
    vote: AggregatedVote,
    current_height: u64,
) -> Result<(), UpgradeError> {
    let role = match &action {
        MultisigAction::Upgrade(upgrade) => upgrade.required_role(),
        MultisigAction::Cancel(cancel) => {
            let target_action_id = cancel.target_id();
            let queued = state
                .find_queued(target_action_id)
                .ok_or(UpgradeError::UnknownAction(*target_action_id))?;
            queued.action().required_role()
        }
        MultisigAction::Enact(enact) => {
            let target_action_id = enact.target_id();
            let queued = state
                .find_committed(target_action_id)
                .ok_or(UpgradeError::UnknownAction(*target_action_id))?;
            queued.action().required_role()
        }
    };

    let authority = state.authority(role).ok_or(UpgradeError::UnknownRole)?;
    authority.validate_action(&action, &vote)?;

    match action {
        MultisigAction::Upgrade(upgrade) => {
            let id = state.next_update_id();
            match upgrade {
                // If the action is a VerifyingKeyUpdate, queue it to support cancellation
                UpgradeAction::VerifyingKey(_) => {
                    let queued_upgrade = QueuedUpgrade::try_new(id, upgrade, current_height)?;
                    state.enqueue(queued_upgrade);
                }
                // For all other actions, directly schedule them for execution
                _ => {
                    let scheduled_upgrade = ScheduledUpgrade::try_new(id, upgrade, current_height)?;
                    state.schedule(scheduled_upgrade);
                }
            }
            state.increment_next_update_id();
        }
        MultisigAction::Cancel(cancel) => {
            state.remove_queued(cancel.target_id());
        }
        MultisigAction::Enact(enact) => {
            state.commit_to_schedule(enact.target_id());
        }
    }

    // Increase the nonce
    let authority = state.authority_mut(role).ok_or(UpgradeError::UnknownRole)?;
    authority.increment_seqno();

    Ok(())
}
