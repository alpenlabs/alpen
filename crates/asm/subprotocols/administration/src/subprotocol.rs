use strata_asm_common::{
    AnchorState, AsmError, MsgRelayer, NullMsg, Subprotocol, SubprotocolId, TxInputRef,
};
use strata_asm_proto_administration_txs::{
    constants::ADMINISTRATION_SUBPROTOCOL_ID, parser::parse_tx_multisig_action_and_vote,
};

use crate::{
    config::AdministrationSubprotoParams,
    handler::{handle_action, handle_pending_updates},
    state::AdministrationSubprotoState,
};

#[derive(Debug)]
pub struct AdministrationSubprotocol;

impl Subprotocol for AdministrationSubprotocol {
    const ID: SubprotocolId = ADMINISTRATION_SUBPROTOCOL_ID;

    type Params = AdministrationSubprotoParams;

    type State = AdministrationSubprotoState;

    type Msg = NullMsg<0>;

    type AuxInput = ();

    fn init(params: &Self::Params) -> Result<AdministrationSubprotoState, AsmError> {
        Ok(AdministrationSubprotoState::new(params))
    }

    fn process_txs(
        state: &mut AdministrationSubprotoState,
        txs: &[TxInputRef<'_>],
        anchor_pre: &AnchorState,
        _aux_input: &Self::AuxInput,
        relayer: &mut impl MsgRelayer,
        params: &Self::Params,
    ) {
        // Get the current height
        let current_height = anchor_pre.chain_view.pow_state.last_verified_block.height() + 1;

        // Before processing the transactions, we process any queued actions
        handle_pending_updates(state, relayer, current_height);

        for tx in txs {
            if let Ok((action, vote)) = parse_tx_multisig_action_and_vote(tx) {
                let _ = handle_action(state, action, vote, current_height, relayer, params);
            }
        }
    }

    fn process_msgs(
        _state: &mut AdministrationSubprotoState,
        _msgs: &[Self::Msg],
        _params: &Self::Params,
    ) {
    }
}
