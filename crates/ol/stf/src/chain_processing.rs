//! General bookkeeping to ensure that the chain evolves correctly.

use strata_identifiers::{EpochCommitment, OLBlockId};
use strata_ledger_types::{IL1ViewState, StateAccessor};

use crate::{
    context::EpochInitialContext,
    errors::{ExecError, ExecResult},
};

/// Preliminary processing we do at the start of every epoch.
///
/// This is done outside of the checked DA range.
pub fn process_epoch_initial<S: StateAccessor>(
    state: &mut S,
    context: &EpochInitialContext,
) -> ExecResult<()> {
    let estate = state.l1_view_mut();

    // 1. Check that this is the first block of the epoch.
    // TODO maybe we actually do this implicitly?

    // 2. Update the epoch field and insert its commitment into the MMR.
    let state_cur_epoch = estate.cur_epoch();
    let state_next_epoch = state_cur_epoch + 1;
    let block_cur_epoch = context.cur_epoch() as u32;
    if block_cur_epoch != state_next_epoch {
        return Err(ExecError::ChainIntegrity);
    }

    estate.set_cur_epoch(block_cur_epoch);

    // TODO sanity check this works for the genesis block
    let prev_ec = EpochCommitment::from_terminal(state_cur_epoch as u64, context.prev_terminal());

    // TODO insert into MMR

    Ok(())
}
