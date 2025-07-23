//! Execution Environment upgrade transaction handler
//!
//! Handles EE upgrade transactions that update the asm verifying keys and sequencer keys.

use strata_asm_common::{MsgRelayer, TxInputRef};

use crate::{CoreOLState, error::*};

/// Handles execution environment upgrade transactions
///
/// [PLACE_HOLDER] => Waiting for Design and Specification of upgrade transaction
pub(crate) fn handle(
    _state: &mut CoreOLState,
    _tx: &TxInputRef<'_>,
    _relayer: &mut impl MsgRelayer,
) -> Result<()> {
    // TODO: Implement execution environment upgrade transaction handling
    // For now, return an error to be clear about missing functionality
    Err(CoreError::TxParsingError(
        "EE upgrade not yet implemented".to_string(),
    ))
}
