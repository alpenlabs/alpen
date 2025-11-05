use strata_acct_types::{AccountId, BitcoinAmount};
use strata_ledger_types::{
    AccountTypeState, IAccountState, IL1ViewState, ISnarkAccountState, StateAccessor,
};
use strata_ol_chain_types_new::{
    L1Update, OLBlock, OLBlockHeader, OLLog, OLTransaction, TransactionPayload,
};
use strata_params::RollupParams;
use strata_snark_acct_types::SnarkAccountUpdateWithMmrProofs;

use crate::{
    ExecOutput,
    asm::process_asm_log,
    error::{StfError, StfResult},
    post_exec_block_validate, pre_exec_block_validate,
    update::apply_update_outputs,
    verification::verify_update_correctness,
};

/// Processes an OL block. Also performs epoch sealing if the block is terminal.
pub fn execute_block<S: StateAccessor>(
    params: RollupParams,
    state_accessor: &mut S,
    prev_header: OLBlockHeader,
    block: OLBlock,
) -> StfResult<ExecOutput> {
    // Do some pre execution validation
    pre_exec_block_validate(state_accessor, &block, &prev_header, &params)
        .map_err(StfError::BlockValidation)?;

    let exec_output = execute_block_inner(state_accessor, &block)?;

    // Post execution block validation. Checks state root and logs root.
    post_exec_block_validate::<S>(&block, exec_output.state_root(), exec_output.logs())
        .map_err(StfError::BlockValidation)?;

    Ok(exec_output)
}

/// Block execution without validation. Used by block assembly.
pub fn execute_block_inner<S: StateAccessor>(
    state_accessor: &mut S,
    block: &OLBlock,
) -> StfResult<ExecOutput> {
    let mut stf_logs = Vec::new();
    // Execute transactions.
    for tx in block.body().txs() {
        let logs = execute_transaction(state_accessor, tx)?;
        stf_logs.extend_from_slice(&logs);
    }

    let _pre_seal_root = state_accessor.compute_state_root();

    // Check if needs to seal epoch
    if let Some(l1update) = block.body().l1_update() {
        let seal_logs = seal_epoch(state_accessor, l1update)?;
        stf_logs.extend_from_slice(&seal_logs);

        // Increment the current epoch now that we've processed the terminal block.
        let cur_epoch = state_accessor.l1_view().cur_epoch();
        let new_epoch = cur_epoch
            .checked_add(1)
            .ok_or(StfError::EpochOverflow { cur_epoch })?;
        state_accessor.l1_view_mut().set_cur_epoch(new_epoch);
    }

    let new_root = state_accessor.compute_state_root();
    let out = ExecOutput::new(new_root, stf_logs);
    Ok(out)
}

fn seal_epoch(
    state_accessor: &mut impl StateAccessor,
    l1update: &L1Update,
) -> StfResult<Vec<OLLog>> {
    let mut logs = Vec::new();
    let l1_view = state_accessor.l1_view();
    let mut cur_height = l1_view.last_l1_height();
    let mut cur_blkid = (*l1_view.last_l1_blkid()).into();

    for manifest in &l1update.manifests {
        for log in manifest.logs() {
            logs.extend_from_slice(&process_asm_log(state_accessor, log)?);
        }
        // TODO: Insert into witness mmr
        cur_height += 1;
        cur_blkid = manifest.l1_blkid();
    }

    let l1view = state_accessor.l1_view_mut();
    l1view.set_last_l1_blkid(cur_blkid.into());
    l1view.set_last_l1_height(cur_height);

    Ok(logs)
}

fn execute_transaction<S: StateAccessor>(
    state_accessor: &mut S,
    tx: &OLTransaction,
) -> StfResult<Vec<OLLog>> {
    let Some(target) = tx.payload().target() else {
        // TODO: should we do anything?
        return Ok(Vec::new());
    };
    let Some(mut acct_state) = state_accessor.get_account_state(target)?.cloned() else {
        return Err(StfError::NonExistentAccount(target));
    };

    let (logs, output_value) = match tx.payload() {
        TransactionPayload::SnarkAccountUpdate { target, update } => {
            let type_state = acct_state.get_type_state()?;
            let cur_balance = acct_state.balance();
            if let AccountTypeState::Snark(mut snark_state) = type_state {
                let logs = process_snark_update(
                    state_accessor,
                    *target,
                    &mut snark_state,
                    update,
                    cur_balance,
                )?;
                (
                    logs,
                    update.update().operation().outputs().compute_total_value(),
                )
            } else {
                (Vec::new(), Some(BitcoinAmount::zero()))
            }
        }
        TransactionPayload::GenericAccountMessage { .. } => {
            return Err(StfError::UnsupportedTransaction);
        }
    };
    // Update balance
    let total_sent = output_value.ok_or(StfError::BitcoinAmountOverflow)?;

    let _coins = acct_state.take_balance(total_sent);
    // TODO: do something with coins

    state_accessor.update_account_state(target, acct_state)?;

    Ok(logs)
}

fn process_snark_update<S: StateAccessor>(
    state_accessor: &mut S,
    target: AccountId,
    snark_state: &mut impl ISnarkAccountState,
    update: &SnarkAccountUpdateWithMmrProofs,
    cur_balance: BitcoinAmount,
) -> StfResult<Vec<OLLog>> {
    let verified_update =
        verify_update_correctness(state_accessor, target, snark_state, update, cur_balance)?;
    let new_state = verified_update.operation().new_state();
    let seq_no = verified_update.operation().seq_no();

    // Apply update outputs.
    let logs = apply_update_outputs(state_accessor, target, verified_update)?;

    // After applying updates, update the proof state.
    snark_state.set_proof_state_directly(
        new_state.inner_state(),
        new_state.next_inbox_msg_idx(),
        seq_no,
    );
    Ok(logs)
}
