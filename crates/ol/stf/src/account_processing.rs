//! Account-specific interaction handling, such as messages.

use strata_acct_types::{AccountId, MsgPayload};
use strata_ledger_types::StateAccessor;

use crate::{
    context::SlotExecContext,
    errors::{ExecError, ExecResult},
};

/// Processes a message by delivering it to its destination, which might involve
/// touching the ledger state.
pub(crate) fn process_message<S: StateAccessor>(
    state: &mut S,
    sender: AccountId,
    target: AccountId,
    msg: MsgPayload,
    context: &mut SlotExecContext,
) -> ExecResult<()> {
    // TODO
    Ok(())
}
