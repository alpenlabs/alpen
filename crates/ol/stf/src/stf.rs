use strata_acct_types::{AccountId, AcctError, BitcoinAmount};
use strata_ledger_types::{
    AccountTypeState, IAccountState, IL1ViewState, ISnarkAccountState, StateAccessor,
};
use strata_ol_chain_types_new::{
    L1Update, LogEmitter, OLBlock, OLBlockBody, OLLog, OLTransaction, TransactionPayload,
};
use strata_snark_acct_sys as snark_sys;
use strata_snark_acct_types::{SnarkAccountUpdateContainer, UpdateOperationData};

use crate::{
    ExecOutput,
    asm::process_asm_log,
    context::BlockExecContext,
    error::{StfError, StfResult},
    ledger::LedgerInterfaceImpl,
    post_exec_block_validate, pre_exec_block_validate,
};

/// Executes an OL block with full validation.
///
/// Performs pre-execution validation (header checks), executes transactions,
/// handles epoch sealing for terminal blocks, and validates post-state root.
///
/// Returns execution output containing the new state root and accumulated logs.
pub fn execute_block<S: StateAccessor>(
    ctx: BlockExecContext,
    state_accessor: &mut S,
    block: OLBlock,
) -> StfResult<ExecOutput> {
    // Do some pre execution validation
    pre_exec_block_validate(state_accessor, &block, ctx.prev_header(), ctx.params())
        .map_err(StfError::BlockValidation)?;

    let exec_output = execute_block_body(ctx, state_accessor, block.body())?;

    // Post execution block validation. Checks state root and logs root.
    post_exec_block_validate::<S>(&block, exec_output.state_root(), exec_output.logs())
        .map_err(StfError::BlockValidation)?;

    Ok(exec_output)
}

/// Executes block body without any block level validation.
///
/// Used by block assembly where validation isn't needed. Executes all transactions
/// and handles epoch sealing but skips header and state root validation.
pub fn execute_block_body<S: StateAccessor>(
    ctx: BlockExecContext,
    state_accessor: &mut S,
    block_body: &OLBlockBody,
) -> StfResult<ExecOutput> {
    // Execute transactions.
    for tx in block_body.txs() {
        execute_transaction(&ctx, state_accessor, tx)?;
    }

    let _pre_seal_root = state_accessor.compute_state_root();

    // Check if needs to seal epoch, i.e is a terminal block.
    if let Some(l1update) = block_body.l1_update() {
        seal_epoch(&ctx, state_accessor, l1update)?;

        // Increment the current epoch now that we've processed the terminal block.
        let cur_epoch = state_accessor.l1_view().cur_epoch();
        let new_epoch = cur_epoch
            .checked_add(1)
            .ok_or(StfError::EpochOverflow { cur_epoch })?;
        state_accessor.l1_view_mut().set_cur_epoch(new_epoch);
    }

    let new_root = state_accessor.compute_state_root();
    let out = ExecOutput::new(new_root, ctx.into_logs());
    Ok(out)
}

fn seal_epoch(
    ctx: &BlockExecContext,
    state_accessor: &mut impl StateAccessor,
    l1update: &L1Update,
) -> StfResult<()> {
    let l1_view = state_accessor.l1_view();
    let mut cur_height = l1_view.last_l1_height();
    let mut cur_blkid = (*l1_view.last_l1_blkid()).into();

    for manifest in &l1update.manifests {
        for log in manifest.logs() {
            process_asm_log(ctx, state_accessor, log)?;
        }
        // TODO: Insert into witness mmr
        cur_height += 1;
        cur_blkid = manifest.l1_blkid();
    }

    let l1view = state_accessor.l1_view_mut();
    l1view.set_last_l1_blkid(cur_blkid.into());
    l1view.set_last_l1_height(cur_height);

    Ok(())
}

fn execute_transaction<S: StateAccessor>(
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

    let output_value = match tx.payload() {
        TransactionPayload::SnarkAccountUpdate { target, update } => {
            let type_state = acct_state.get_type_state()?;
            let cur_balance = acct_state.balance();
            if let AccountTypeState::Snark(mut snark_state) = type_state {
                process_snark_update(
                    ctx,
                    state_accessor,
                    *target,
                    &mut snark_state,
                    update,
                    cur_balance,
                )?;
                update
                    .base_update()
                    .operation()
                    .outputs()
                    .compute_total_value()
            } else {
                Some(BitcoinAmount::zero())
            }
        }
        TransactionPayload::GenericAccountMessage { .. } => {
            return Err(StfError::UnsupportedTransaction);
        }
    };
    // Update balance
    let total_sent = output_value.ok_or(AcctError::BitcoinAmountOverflow)?;

    let coins = acct_state.take_balance(total_sent)?;

    coins.safely_consume_unchecked(); // not sure of the benefits of doing this here. It would be
    // nice to have some kind of compile-time safety

    state_accessor.update_account_state(target, acct_state)?;

    Ok(())
}

/// Processes a snark account update: verification → output application → state update.
///
/// Creates a [`LedgerRef`] to apply outputs, which delegates to `send_message`/`send_transfer`
/// for handling transfers to other accounts.
fn process_snark_update<S: StateAccessor>(
    ctx: &BlockExecContext,
    state_accessor: &mut S,
    target: AccountId,
    snark_state: &mut impl ISnarkAccountState,
    update: &SnarkAccountUpdateContainer,
    cur_balance: BitcoinAmount,
) -> StfResult<()> {
    let verified_update = snark_sys::verify_update_correctness(
        state_accessor,
        target,
        snark_state,
        update,
        cur_balance,
    )?;
    let new_state = verified_update.operation().new_state();
    let seq_no = verified_update.operation().seq_no();
    let operation = verified_update.operation().clone();

    // Create ledger ref
    let mut ledger_ref = LedgerInterfaceImpl::new(target, state_accessor, ctx);

    // Apply update outputs.
    snark_sys::apply_update_outputs(&mut ledger_ref, verified_update)?;

    // After applying updates, update the proof state.
    snark_state.set_proof_state_directly(
        new_state.inner_state(),
        new_state.next_inbox_msg_idx(),
        seq_no,
    );

    // Construct and emit SnarkUpdate Log.
    let log = construct_update_log(target, operation);
    LogEmitter::emit_log(ctx, log);

    Ok(())
}

fn construct_update_log(target: AccountId, operation: UpdateOperationData) -> OLLog {
    let log_extra = vec![];
    let to_idx = operation.new_state().next_inbox_msg_idx() - 1;
    let from_idx = to_idx - operation.processed_messages().len() as u64 + 1;

    OLLog::snark_account_update(
        target,
        from_idx,
        to_idx,
        operation.new_state().inner_state().into(),
        log_extra,
    )
}
