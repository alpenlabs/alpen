//! Transaction handlers for the Core subprotocol
//!
//! This module contains handlers for different transaction types processed by the Core subprotocol.

pub(crate) mod checkpoint;
pub(crate) mod forced_inclusion;
pub(crate) mod upgrade;

use strata_asm_common::{AnchorState, MsgRelayer, Subprotocol, TxInputRef};

use crate::{
    CoreOLState, OLCoreSubproto,
    constants::{EE_UPGRADE_TX_TYPE, FORCED_INCLUSION_TX_TYPE, OL_STF_CHECKPOINT_TX_TYPE},
    error::*,
};

/// Routes transactions to appropriate handlers based on transaction type
pub(crate) fn route_transaction(
    state: &mut CoreOLState,
    tx: &TxInputRef<'_>,
    _anchor_pre: &AnchorState,
    _aux_inputs: &[<OLCoreSubproto as Subprotocol>::AuxInput],
    relayer: &mut impl MsgRelayer,
) -> Result<()> {
    // [PLACE_HOLDER]
    // TODO: Define the role of anchor_pre and aux_inputs in checkpoint validation logic and update
    // the code accordingly
    match tx.tag().tx_type() {
        OL_STF_CHECKPOINT_TX_TYPE => checkpoint::handle(state, tx, relayer),
        FORCED_INCLUSION_TX_TYPE => forced_inclusion::handle(state, tx, relayer),
        EE_UPGRADE_TX_TYPE => upgrade::handle(state, tx, relayer),
        _ => Err(CoreError::TxParsingError("unsupported tx type".to_string())),
    }
}
