//! Forced inclusion transaction handler
//!
//! Handles forced inclusion transactions that allow users to bypass sequencer censorship.

use strata_asm_common::{MsgRelayer, TxInputRef};

use crate::{CoreOLState, error::*};

/// Handles forced inclusion transactions
///
/// [PLACE_HOLDER] => Waiting for Design and Specification of forced inclusion transaction
pub(crate) fn handle(
    _state: &mut CoreOLState,
    _tx: &TxInputRef<'_>,
    _relayer: &mut impl MsgRelayer,
) -> Result<()> {
    // TODO: Implement forced inclusion transaction handling
    // For now, return an error to be clear about missing functionality
    Err(CoreError::TxParsingError(
        "forced inclusion not yet implemented".to_string(),
    ))
}
