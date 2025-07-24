//! Transaction handlers for the Core subprotocol
//!
//! This module contains handlers for different transaction types processed by the Core subprotocol.

pub(crate) mod checkpoint;

use strata_asm_common::{AnchorState, MsgRelayer, Subprotocol, TxInputRef};

use crate::{CoreOLState, OLCoreSubproto, constants::OL_STF_CHECKPOINT_TX_TYPE, error::*};

/// Routes transactions to appropriate handlers based on transaction type
pub(crate) fn route_transaction(
    state: &mut CoreOLState,
    tx: &TxInputRef<'_>,
    anchor_pre: &AnchorState,
    aux_inputs: &[<OLCoreSubproto as Subprotocol>::AuxInput],
    relayer: &mut impl MsgRelayer,
) -> Result<()> {
    // [PLACE_HOLDER]
    // TODO: Define the role of anchor_pre and aux_inputs in checkpoint validation logic and update
    // the code accordingly
    match tx.tag().tx_type() {
        OL_STF_CHECKPOINT_TX_TYPE => {
            checkpoint::handle_checkpoint_transaction(state, tx, relayer, anchor_pre, aux_inputs)
        }
        // [PLACE_HOLDER] Add other transaction types related to vk upgrade, etc.
        _ => Err(CoreError::TxParsingError("unsupported tx type".to_string())),
    }
}
