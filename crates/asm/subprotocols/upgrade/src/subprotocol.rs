use strata_asm_common::{MsgRelayer, NullMsg, Subprotocol, SubprotocolId, TxInput};

use crate::{
    actions::{cancel::handle_cancel_tx, enact::handle_enactment_tx, update::handle_update_tx},
    roles::StrataProof,
    state::UpgradeSubprotoState,
    txs::{CANCEL_TX_TYPE, ENACT_TX_TYPE, updates::UpgradeAction},
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
    ) {
        // Before processing the transactions, we handle any queued actions
        state.tick_and_move_queued_to_committed();

        // Process each transaction based on its type
        for tx in txs {
            match tx.tag().tx_type() {
                CANCEL_TX_TYPE => {
                    let _ = handle_cancel_tx(state, tx);
                }
                ENACT_TX_TYPE => {
                    let _ = handle_enactment_tx(state, tx);
                }
                _ => {
                    let _ = handle_update_tx(state, tx);
                }
            }
        }

        handle_scheduled_actions(state, relayer);
    }

    fn process_msgs(_state: &mut UpgradeSubprotoState, _msgs: &[Self::Msg]) {}
}

fn handle_scheduled_actions(state: &mut UpgradeSubprotoState, _relayer: &mut impl MsgRelayer) {
    // Decrement the blocks_remaining for each pending action
    let actions_to_enact = state.tick_and_collect_ready_actions();

    for action in actions_to_enact {
        match action.action() {
            UpgradeAction::Multisig(update) => {
                state.update_multisig_config(update.role(), update.config_update());
            }
            UpgradeAction::VerifyingKey(update) => match update.proof_kind() {
                StrataProof::ASM => {
                    // Emit Log
                }
                StrataProof::OlStf => {
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
