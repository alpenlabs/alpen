use strata_acct_types::{AccountId, AcctError};
use strata_asm_common::AsmManifest;
use strata_ledger_types::{
    AccountTypeState, IAccountState, IGlobalState, IL1ViewState, ISnarkAccountState, StateAccessor,
};
use strata_ol_chain_types_new::{
    L1BlockCommitment, L1Update, LogEmitter, OLBlock, OLBlockBody, OLLog, OLTransaction,
    TransactionExtra, TransactionPayload,
};
use strata_snark_acct_sys as snark_sys;
use strata_snark_acct_types::{SnarkAccountUpdateContainer, UpdateOperationData};
use tracing::debug;

use crate::{
    ExecOutput,
    asm::process_asm_log,
    context::BlockExecContext,
    error::{StfError, StfResult},
    ledger::LedgerInterfaceImpl,
    post_exec_block_validate, pre_exec_block_validate,
};

/// Executes an OL block with full validation. This modifies the state in-place. If anything beyond
/// that needs to be done like generating write-batches and DA, it needs to be done by the
/// [`StateAccessor`] implementation.
///
/// Performs pre-execution validation (header checks), executes transactions,
/// handles epoch sealing for terminal blocks, and validates post-state root.
///
/// Returns execution output containing the new state root and accumulated logs.
pub fn validate_and_execute_block<S: StateAccessor>(
    ctx: BlockExecContext,
    state_accessor: &mut S,
    block: OLBlock,
) -> StfResult<ExecOutput> {
    // Do some pre execution validation
    pre_exec_block_validate(&block, ctx.prev_header(), ctx.params())
        .map_err(StfError::BlockValidation)?;

    let exec_output = execute_block_body(ctx, state_accessor, block.body())?;

    // Post execution block validation. Checks state root and logs root.
    post_exec_block_validate::<S>(&block, exec_output.state_root(), exec_output.logs())
        .map_err(StfError::BlockValidation)?;

    Ok(exec_output)
}

/// Executes block body assuming pre-validation and post-validation are done before and after this.
fn execute_block_body<S: StateAccessor>(
    ctx: BlockExecContext,
    state_accessor: &mut S,
    block_body: &OLBlockBody,
) -> StfResult<ExecOutput> {
    // Execute transactions.
    execute_transactions(&ctx, state_accessor, block_body.txs())?;

    // Check if needs to seal epoch, i.e is a terminal block.
    if let Some(l1update) = block_body.l1_update() {
        let preseal_root = state_accessor.compute_state_root();

        // Check pre_seal_root matches with l1update preseal_root.
        if l1update.preseal_state_root() != preseal_root {
            return Err(StfError::PresealRootMismatch {
                expected: l1update.preseal_state_root(),
                got: preseal_root,
            });
        }
        seal_epoch(&ctx, state_accessor, l1update)?;
    }

    let new_root = state_accessor.compute_state_root();
    let out = ExecOutput::new(new_root, ctx.into_logs());
    Ok(out)
}

/// Executes the OL transactions and updates the state accordingly.
pub fn execute_transactions(
    ctx: &BlockExecContext,
    state_accessor: &mut impl StateAccessor,
    txs: &[OLTransaction],
) -> StfResult<()> {
    for tx in txs {
        validate_tx_extra(state_accessor, tx.extra())?;
        execute_transaction(ctx, state_accessor, tx)?;
    }
    Ok(())
}

pub fn validate_tx_extra(
    state_accessor: &impl StateAccessor,
    extra: &TransactionExtra,
) -> StfResult<()> {
    let cur_slot = state_accessor.global().cur_slot();
    if let Some(min_slot) = extra.min_slot()
        && min_slot > cur_slot
    {
        return Err(StfError::InvalidTxExtra);
    }
    if let Some(max_slot) = extra.max_slot()
        && max_slot < cur_slot
    {
        return Err(StfError::InvalidTxExtra);
    }
    Ok(())
}

pub fn seal_epoch(
    ctx: &BlockExecContext,
    state_accessor: &mut impl StateAccessor,
    l1update: &L1Update,
) -> StfResult<()> {
    let l1blk_commt: L1BlockCommitment =
        process_asm_manifests(ctx, state_accessor, l1update.manifests())?;

    let l1view = state_accessor.l1_view_mut();
    let blkid = *(l1blk_commt.blkid());
    l1view.set_last_l1_blkid(blkid);
    l1view.set_last_l1_height(l1blk_commt.height());

    // Increment the current epoch now that we've sealed the epoch
    let cur_epoch = state_accessor.l1_view().cur_epoch();
    let new_epoch = cur_epoch
        .checked_add(1)
        .ok_or(StfError::EpochOverflow { cur_epoch })?;
    state_accessor.l1_view_mut().set_cur_epoch(new_epoch);

    Ok(())
}

/// Processes the ASM Manifests and returns the latest l1 commitment in the manifests updating the
/// state accordingly.
pub fn process_asm_manifests(
    ctx: &BlockExecContext,
    state_accessor: &mut impl StateAccessor,
    manifests: &[AsmManifest],
) -> StfResult<L1BlockCommitment> {
    let l1_view = state_accessor.l1_view();
    let mut cur_height = l1_view.last_l1_height();
    let mut cur_blkid = *l1_view.last_l1_blkid();

    for manifest in manifests {
        for log in manifest.logs() {
            process_asm_log(ctx, state_accessor, log)?;
        }

        // Append manifest
        let l1_view_mut = state_accessor.l1_view_mut();
        l1_view_mut.append_manifest(manifest.clone());

        cur_height += 1;
        cur_blkid = *manifest.blkid();
    }

    Ok(L1BlockCommitment::new(cur_height, cur_blkid))
}

/// Executes the transaction modifying the state accessor. The modifications to the state should be
/// atomic, i.e. if at any point during the execution it fails, the changes need to be reverted from
/// the state.
pub(crate) fn execute_transaction<S: StateAccessor>(
    ctx: &BlockExecContext,
    state_accessor: &mut S,
    tx: &OLTransaction,
) -> StfResult<()> {
    let Some(target) = tx.payload().target() else {
        // TODO: should we do anything?
        return Ok(());
    };
    let Some(mut acct_state) = state_accessor.get_account_state(target)?.cloned() else {
        return Err(AcctError::NonExistentAccount(target).into());
    };

    // TODO: SELF-SEND BUG - We clone the account state here, then pass it to the handler.
    // If the handler (via apply_update_outputs) sends coins/messages back to `target`,
    // those writes go to state_accessor. Then we overwrite with the clone below, losing
    // the self-send changes. Fix options:
    // 1. Don't clone - work directly with state in accessor (requires refactoring borrow checker)
    // 2. Buffer self-sends and apply after handler returns
    // 3. Detect and explicitly fail on self-sends

    match tx.payload() {
        TransactionPayload::SnarkAccountUpdate { target, update } => {
            process_snark_update(ctx, state_accessor, *target, &mut acct_state, update)?;
        }
        TransactionPayload::GenericAccountMessage { .. } => {
            return Err(StfError::UnsupportedTransaction);
        }
    };

    state_accessor.update_account_state(target, acct_state)?;

    Ok(())
}

/// Processes a snark account update: verification → output application → state update.
///
/// Creates a [`LedgerInterfaceImpl`] to apply outputs, which delegates to
/// `send_message`/`send_transfer` for handling transfers to other accounts.
fn process_snark_update<S: StateAccessor>(
    ctx: &BlockExecContext,
    state_accessor: &mut S,
    target: AccountId,
    acct_state: &mut impl IAccountState,
    update: &SnarkAccountUpdateContainer,
) -> StfResult<()> {
    // Extract snark state and verify it's the right type
    let type_state = acct_state.get_type_state()?;
    let AccountTypeState::Snark(mut snark_state) = type_state else {
        return Err(StfError::SnarkUpdateForNonSnarkAccount(target));
    };

    let cur_balance = acct_state.balance();

    let verified_update = snark_sys::verify_update_correctness(
        state_accessor,
        target,
        &snark_state,
        update,
        cur_balance,
    )?;
    let new_state = verified_update.operation().new_state();
    let seq_no = verified_update.operation().seq_no();
    let operation = verified_update.operation().clone();

    // Calculate total output value and deduct from balance
    let total_sent = update
        .base_update()
        .operation()
        .outputs()
        .compute_total_value()
        .ok_or(AcctError::BitcoinAmountOverflow)?;

    let coins = acct_state.take_balance(total_sent)?;
    coins.safely_consume_unchecked();

    // Create ledger impl
    let mut ledger_impl = LedgerInterfaceImpl::new(target, state_accessor, ctx);

    // Apply update outputs.
    snark_sys::apply_update_outputs(&mut ledger_impl, verified_update)?;

    // After applying updates, update the proof state.
    snark_state.set_proof_state_directly(
        new_state.inner_state(),
        new_state.next_inbox_msg_idx(),
        seq_no,
    );

    // Write the updated snark state back to the account state
    acct_state.set_type_state(AccountTypeState::Snark(snark_state))?;

    // Construct and emit SnarkUpdate Log.
    let log = construct_update_log(target, operation);
    debug!(?log, "emitting snark update log");
    ctx.emit_log(log);

    Ok(())
}

fn construct_update_log(target: AccountId, operation: UpdateOperationData) -> OLLog {
    let log_extra = operation.extra_data().to_vec();
    let next_msg_idx = operation.new_state().next_inbox_msg_idx();

    OLLog::snark_account_update(
        target,
        next_msg_idx,
        operation.new_state().inner_state().into(),
        log_extra,
    )
}
