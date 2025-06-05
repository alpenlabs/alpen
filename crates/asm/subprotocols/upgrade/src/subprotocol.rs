use strata_asm_common::{MsgRelayer, NullMsg, Subprotocol, SubprotocolId, TxInput};

use crate::{
    actions::{
        cancel::{CANCEL_TX_TYPE, handle_cancel_action},
        multisig_update::{MULTISIG_CONFIG_UPDATE_TX_TYPE, handle_multisig_config_update},
        operator_update::{OPERATOR_UPDATE_TX_TYPE, handle_operator_update},
        seq_update::{SEQUENCER_UPDATE_TX_TYPE, handle_sequencer_update},
        vk_update::{VK_UPDATE_TX_TYPE, handle_vk_update},
    },
    state::UpgradeSubprotoState,
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
