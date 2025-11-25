//! General bookkeeping to ensure that the chain evolves correctly.

use strata_identifiers::{EpochCommitment, OLBlockId};
use strata_ledger_types::{IGlobalState, IL1ViewState, StateAccessor};

use crate::{
    context::{BlockContext, EpochInitialContext},
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

    // 2. Make sure the state's epoch matches the block.
    let state_cur_epoch = estate.cur_epoch();
    let block_cur_epoch = context.cur_epoch() as u32;
    if block_cur_epoch != state_cur_epoch {
        return Err(ExecError::ChainIntegrity);
    }

    // 3. Insert the previous terminal info into the MMR.
    // For genesis block (epoch 0), there is no previous terminal
    if state_cur_epoch > 0 {
        let prev_ec =
            EpochCommitment::from_terminal(state_cur_epoch as u64 - 1, context.prev_terminal());
        // TODO insert into MMR
    }

    Ok(())
}

/// Processing that happens at the start of every block.
///
/// This updates the global state to track the current slot number.
pub fn process_block_start<S: StateAccessor>(
    state: &mut S,
    context: &BlockContext<'_>,
) -> ExecResult<()> {
    // 1. Make sure that our epoch matches what we expect it to be based on the
    // previous header.
    // FIXME we already basically do this in verify_header_continuity, should we
    // also error out on this when constructing blocks?
    let header_epoch = context.epoch();
    if let Some(ph) = context.parent_header() {
        let exp_epoch = ph.epoch() + ph.is_terminal() as u32;
        if context.epoch() != exp_epoch {
            return Err(ExecError::IncorrectEpoch(
                ph.epoch(),
                context.epoch(),
                ph.is_terminal(),
            ));
        }
    } else {
        if context.slot() != 0 || context.epoch() != 0 {
            return Err(ExecError::GenesisCoordsNonzero);
        }
    }

    // 2. Make sure that the current state epoch matches the header's epoch.
    let state_epoch = state.l1_view().cur_epoch();
    if header_epoch != state_epoch {
        return Err(ExecError::EpochMismatch(header_epoch, state_epoch));
    }

    // 3. Update the global state's current slot to match the block's slot
    let slot = context.slot();
    state.global_mut().set_cur_slot(slot);

    Ok(())
}
