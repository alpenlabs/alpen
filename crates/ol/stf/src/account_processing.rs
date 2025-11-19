//! Account-specific interaction handling, such as messages.

use strata_acct_types::{AccountId, MsgPayload};
use strata_ledger_types::StateAccessor;

use crate::{
    context::BasicExecContext,
    errors::{ExecError, ExecResult},
};

/// Processes a message by delivering it to its destination, which might involve
/// touching the ledger state.
///
/// This takes a [`EpochContext`] because messages can be issued both in regular
/// block processing and at epoch sealing.
pub(crate) fn process_message<S: StateAccessor>(
    state: &mut S,
    sender: AccountId,
    target: AccountId,
    msg: MsgPayload,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    // TODO
    Ok(())
}
