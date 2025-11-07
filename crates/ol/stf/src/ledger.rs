use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload};
use strata_ledger_types::StateAccessor;
use strata_snark_acct_types::LedgerInterface;

use crate::{
    context::BlockExecContext,
    error::StfError,
    update::{send_message, send_transfer},
};

/// Adapts the STF's send_message/send_transfer functions to the [`LedgerInterface`] trait.
///
/// This allows snark-acct-sys to apply update outputs without depending on ol/stf directly.
/// Represents an account's view of the ledger for sending funds.
///
/// Note: Not `Clone` because it contains `&mut S`.
// FIXME: a better name.
#[derive(Debug)]
pub(crate) struct LedgerRef<'a, S: StateAccessor> {
    acct_id: AccountId,
    state_accessor: &'a mut S,
    ctx: &'a BlockExecContext,
}

impl<'a, S: StateAccessor> LedgerRef<'a, S> {
    pub(crate) fn new(
        acct_id: AccountId,
        state_accessor: &'a mut S,
        ctx: &'a BlockExecContext,
    ) -> Self {
        Self {
            acct_id,
            state_accessor,
            ctx,
        }
    }
}

impl<'a, S: StateAccessor> LedgerInterface<StfError> for LedgerRef<'a, S> {
    fn send_message(&mut self, dest: AccountId, payload: MsgPayload) -> Result<(), StfError> {
        send_message(self.ctx, self.state_accessor, self.acct_id, dest, &payload)
    }

    fn send_transfer(&mut self, dest: AccountId, value: BitcoinAmount) -> Result<(), StfError> {
        send_transfer(self.ctx, self.state_accessor, self.acct_id, dest, value)
    }
}
