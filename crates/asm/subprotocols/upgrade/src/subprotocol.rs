use strata_asm_common::{MsgRelayer, NullMsg, Subprotocol, SubprotocolId, TxInput};

use crate::{
    actions::{
        cancel::handle_cancel_action, multisig_update::handle_multisig_config_update,
        operator_update::handle_operator_update, seq_update::handle_sequencer_update,
        vk_update::handle_vk_update,
    },
    state::UpgradeSubprotoState,
};

pub const UPGRADE_SUBPROTOCOL_ID: u8 = 0;

pub const MULTISIG_CONFIG_UPDATE_TX_TYPE: u8 = 1;
pub const VK_UPDATE_TX_TYPE: u8 = 2;
pub const OPERATOR_UPDATE_TX_TYPE: u8 = 3;
pub const SEQUENCER_UPDATE_TX_TYPE: u8 = 4;
pub const CANCEL_TX_TYPE: u8 = 5;

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
        for tx in txs {
            match tx.tag().tx_type() {
                MULTISIG_CONFIG_UPDATE_TX_TYPE => {
                    let _ = handle_multisig_config_update(state, tx, relayer);
                }
                VK_UPDATE_TX_TYPE => {
                    let _ = handle_vk_update(state, tx, relayer);
                }
                OPERATOR_UPDATE_TX_TYPE => {
                    let _ = handle_operator_update(state, tx, relayer);
                }
                SEQUENCER_UPDATE_TX_TYPE => {
                    let _ = handle_sequencer_update(state, tx, relayer);
                }
                CANCEL_TX_TYPE => {
                    let _ = handle_cancel_action(state, tx, relayer);
                }
                _ => {}
            }
        }
    }

    fn process_msgs(state: &mut UpgradeSubprotoState, msgs: &[Self::Msg]) {}
}
