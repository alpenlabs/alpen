use strata_ledger_types::StateAccessor;
use strata_ol_chain_types_new::{
    L1Update, OLBlock, OLBlockHeader, OLLog, OLTransaction, TransactionPayload, TxTypeId,
};
use strata_ol_state_types::OLState;
use strata_primitives::{Buf32, params::RollupParams};

use crate::{
    asm::process_asm_log,
    error::{StfError, StfResult},
    post_exec_validation, pre_exec_block_validate,
    update::{apply_update_outputs, verify_update_correctness},
};

/// Processes an OL block. Also performs epoch sealing if the block is terminal.
pub fn execute_block(
    params: RollupParams,
    state_accessor: &mut impl StateAccessor<GlobalState = OLState>,
    prev_header: OLBlockHeader,
    block: OLBlock,
) -> StfResult<ExecOutput> {
    // Do some pre execution validation
    pre_exec_block_validate(&block, &prev_header, &params).map_err(StfError::BlockValidation)?;

    let mut stf_logs = Vec::new();

    // Execute transactions.
    for tx in block.body().txs() {
        let logs = execute_transaction(state_accessor, tx)?;
        stf_logs.extend_from_slice(&logs);
    }

    let _pre_seal_root = state_accessor.global().compute_root();

    // Check if needs to seal epoch
    if let Some(l1update) = block.body().l1_update() {
        let seal_logs = seal_epoch(state_accessor, l1update)?;
        stf_logs.extend_from_slice(&seal_logs);

        // Increment the current epoch now that we've processed the terminal block.
        let cur_epoch = state_accessor.global().cur_epoch();
        state_accessor.global_mut().set_cur_epoch(cur_epoch + 1);
    }

    let new_state = state_accessor.global().to_owned();

    // Post execution block validation. Checks state root and logs root.
    post_exec_validation(&block, &new_state, &stf_logs).map_err(StfError::BlockValidation)?;

    let new_root = new_state.compute_root();
    Ok(ExecOutput::new(new_root, stf_logs))
}

fn seal_epoch(
    state_accessor: &mut impl StateAccessor<GlobalState = OLState>,
    l1update: &L1Update,
) -> StfResult<Vec<OLLog>> {
    let mut logs = Vec::new();
    let state = state_accessor.global();
    let mut cur_height = state.l1_view().block_height();
    let mut cur_blkid = state.l1_view().block_id();

    for manifest in &l1update.manifests {
        for log in manifest.logs() {
            logs.extend_from_slice(&process_asm_log(state_accessor, log)?);
        }
        // TODO: Insert into witness mmr
        cur_height = manifest.l1_blkheight();
        cur_blkid = manifest.l1_blkid();
    }

    let l1view = state_accessor.global_mut().l1_view_mut();
    l1view.set_block_id(cur_blkid);
    l1view.set_block_height(cur_height);

    Ok(logs)
}

fn execute_transaction<S: StateAccessor<GlobalState = OLState>>(
    state_accessor: &mut S,
    tx: &OLTransaction,
) -> StfResult<Vec<OLLog>> {
    match tx.payload() {
        TransactionPayload::SnarkAccountUpdate { target, update } => {
            let Some(mut acct_state) = state_accessor.get_account_state(*target)?.cloned() else {
                return Err(StfError::NonExistentAccount(*target));
            };

            let verified_udpate =
                verify_update_correctness(state_accessor, *target, &acct_state, update)?;
            let logs =
                apply_update_outputs(state_accessor, *target, &mut acct_state, verified_udpate)?;

            state_accessor.update_account_state(*target, acct_state)?;

            Ok(logs)
        }
        TransactionPayload::GenericAccountMessage { .. } => Err(StfError::UnsupportedTransaction),
    }
}

/// Output of a block execution.
#[derive(Clone, Debug)]
pub struct ExecOutput {
    /// The resulting OL state root.
    state_root: Buf32,

    /// The resulting OL logs.
    logs: Vec<OLLog>,
    // TODO: write batch
}

impl ExecOutput {
    pub fn new(state_root: Buf32, logs: Vec<OLLog>) -> Self {
        Self { state_root, logs }
    }

    pub fn state_root(&self) -> Buf32 {
        self.state_root
    }

    pub fn logs(&self) -> &[OLLog] {
        &self.logs
    }
}
