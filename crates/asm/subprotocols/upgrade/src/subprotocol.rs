use strata_asm_common::{
    AnchorState, AsmError, MsgRelayer, NullMsg, Subprotocol, SubprotocolId, TxInputRef,
};
use strata_asm_proto_upgrade_txs::{
    constants::UPGRADE_SUBPROTOCOL_ID, parser::parse_tx_multisig_action_and_vote,
};

use crate::{
    handler::{handle_action, handle_scheduled_updates},
    state::UpgradeSubprotoState,
};

#[derive(Debug)]
pub struct UpgradeSubprotocol;

impl Subprotocol for UpgradeSubprotocol {
    const ID: SubprotocolId = UPGRADE_SUBPROTOCOL_ID;

    type Params = ();

    type State = UpgradeSubprotoState;

    type Msg = NullMsg<0>;

    type AuxInput = ();

    fn init(_params: &Self::Params) -> Result<UpgradeSubprotoState, AsmError> {
        Ok(UpgradeSubprotoState::default())
    }

    fn process_txs(
        state: &mut UpgradeSubprotoState,
        txs: &[TxInputRef<'_>],
        anchor_pre: &AnchorState,
        _aux_input: &Self::AuxInput,
        relayer: &mut impl MsgRelayer,
        _params: &Self::Params,
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

        handle_scheduled_updates(state, relayer, current_height);
    }

    fn process_msgs(
        _state: &mut UpgradeSubprotoState,
        _msgs: &[Self::Msg],
        _params: &Self::Params,
    ) {
    }
}
