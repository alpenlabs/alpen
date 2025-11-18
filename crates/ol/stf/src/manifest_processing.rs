//! ASM manifest processing.

use strata_asm_common::{AsmLogEntry, AsmManifest};
use strata_identifiers::L1Height;
use strata_ledger_types::{IL1ViewState, StateAccessor};
use strata_ol_chain_types_new::OLL1Update;

use crate::{context::SlotExecContext, errors::ExecResult};

/// Processes the L1 update from a block, which is part of the epoch sealing
/// processing.
///
/// This does NOT check the preseal root.
pub fn process_block_l1_update<S: StateAccessor>(
    state: &mut S,
    update: &OLL1Update,
    context: &SlotExecContext,
) -> ExecResult<()> {
    let orig_l1_height = state.l1_view().last_l1_height();
    let mut last = None;

    for (i, mf) in update.manifests().iter().enumerate() {
        let real_height = orig_l1_height + i as u32;
        last = Some((real_height, mf));
        process_asm_manifest(state, real_height, mf, context)?;
    }

    if let Some((last_height, last_mf)) = last {
        // TODO this is where we would update the header, if we want to keep
        // that as defined in the spec
    }

    Ok(())
}

fn process_asm_manifest<S: StateAccessor>(
    state: &mut S,
    real_height: L1Height,
    mf: &AsmManifest,
    context: &SlotExecContext,
) -> ExecResult<()> {
    let estate = state.l1_view();

    // 1. Process each of the logs.
    for log in mf.logs() {
        process_asm_log(state, log, real_height, context)?;
    }

    // 2. Accept the manifest into the ASM MMR.
    state.l1_view_mut().append_manifest(real_height, mf.clone());

    Ok(())
}

fn process_asm_log<S: StateAccessor>(
    state: &mut S,
    log: &AsmLogEntry,
    real_height: L1Height,
    context: &SlotExecContext,
) -> ExecResult<()> {
    // TODO process the logs
    // - log as msgfmt
    // - match on ID
    // - parse body if we recognize it
    // - call out to handler fn below

    Ok(())
}

// temporary defs until we move these to a better place
type DepositLogData = ();
type CheckpointAckLogData = ();

fn process_deposit_log<S: StateAccessor>(
    state: &mut S,
    data: &DepositLogData,
    context: &SlotExecContext,
) -> ExecResult<()> {
    // TODO increment ledger balance, send message off to target with funds
    Ok(())
}

fn process_checkpoint_ack<S: StateAccessor>(
    state: &mut S,
    data: &CheckpointAckLogData,
    context: &SlotExecContext,
) -> ExecResult<()> {
    // TODO update the fields
    Ok(())
}
