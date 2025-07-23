//! Transaction handlers for the Core subprotocol
//!
//! This module contains handlers for different transaction types processed by the Core subprotocol.

pub(crate) mod checkpoint;
pub(crate) mod forced_inclusion;
pub(crate) mod upgrade;

use strata_asm_common::{
    EE_UPGRADE_TX_TYPE, FORCED_INCLUSION_TX_TYPE, MsgRelayer, OL_STF_CHECKPOINT_TX_TYPE, TxInputRef,
};

use crate::{CoreOLState, error::*};

/// Routes transactions to appropriate handlers based on transaction type
pub(crate) fn route_transaction(
    state: &mut CoreOLState,
    tx: &TxInputRef<'_>,
    relayer: &mut impl MsgRelayer,
) -> Result<()> {
    match tx.tag().tx_type() {
        OL_STF_CHECKPOINT_TX_TYPE => checkpoint::handle(state, tx, relayer),
        FORCED_INCLUSION_TX_TYPE => forced_inclusion::handle(state, tx, relayer),
        EE_UPGRADE_TX_TYPE => upgrade::handle(state, tx, relayer),
        _ => Err(CoreError::TxParsingError("unsupported tx type".to_string())),
    }
}
