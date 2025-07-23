//! Forced inclusion transaction handler
//!
//! Handles forced inclusion transactions that allow users to bypass sequencer censorship.

use strata_asm_common::{MsgRelayer, TxInputRef};

use crate::{CoreOLState, error::*};

/// Handles forced inclusion transactions
///
/// TODO: Implement forced inclusion logic
///
/// Forced inclusion allows users to submit transactions directly to L1 that must be
/// included in the next batch, bypassing potential sequencer censorship.
///
/// # Implementation Notes
///
/// This handler should:
/// 1. Parse forced inclusion transaction data
/// 2. Validate transaction format and signatures
/// 3. Queue transaction for inclusion in next batch
/// 4. Update state to track forced inclusion requests
/// 5. Emit logs for monitoring
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
