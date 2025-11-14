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
    pre_exec_block_validate(&block, ctx.prev_header(), ctx.params())
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
        // validate tx extra
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

#[cfg(test)]
mod tests {
    use strata_ledger_types::{IGlobalState, StateAccessor};
    use strata_ol_chain_types_new::TransactionExtra;

    use super::*;

    // Minimal mock for testing validate_tx_extra
    #[derive(Default, Clone)]
    struct MockGlobalState {
        cur_slot: u64,
    }

    impl IGlobalState for MockGlobalState {
        fn cur_slot(&self) -> u64 {
            self.cur_slot
        }

        fn set_cur_slot(&mut self, slot: u64) {
            self.cur_slot = slot;
        }
    }

    #[derive(Clone)]
    struct StubL1ViewState;

    impl strata_ledger_types::IL1ViewState for StubL1ViewState {
        fn cur_epoch(&self) -> strata_primitives::Epoch {
            0
        }

        fn set_cur_epoch(&mut self, _epoch: strata_primitives::Epoch) {}

        fn last_l1_blkid(&self) -> &strata_primitives::L1BlockId {
            unimplemented!()
        }

        fn set_last_l1_blkid(&mut self, _blkid: strata_primitives::L1BlockId) {}

        fn last_l1_height(&self) -> u32 {
            0
        }

        fn set_last_l1_height(&mut self, _height: u32) {}

        fn append_manifest(&mut self, _mf: strata_asm_common::AsmManifest) {}

        fn asm_manifests_mmr(&self) -> &strata_acct_types::Mmr64 {
            unimplemented!()
        }

        fn asm_recorded_epoch(&self) -> &strata_primitives::EpochCommitment {
            unimplemented!()
        }

        fn set_asm_recorded_epoch(&mut self, _epoch: strata_primitives::EpochCommitment) {}

        fn total_ledger_balance(&self) -> strata_acct_types::BitcoinAmount {
            0.into()
        }

        fn increment_total_ledger_balance(
            &mut self,
            _amt: strata_acct_types::BitcoinAmount,
        ) -> strata_acct_types::BitcoinAmount {
            0.into()
        }

        fn decrement_total_ledger_balance(
            &mut self,
            _amt: strata_acct_types::BitcoinAmount,
        ) -> strata_acct_types::BitcoinAmount {
            0.into()
        }
    }

    struct MinimalStateAccessor {
        global: MockGlobalState,
        l1_view: StubL1ViewState,
    }

    impl MinimalStateAccessor {
        fn with_slot(slot: u64) -> Self {
            Self {
                global: MockGlobalState { cur_slot: slot },
                l1_view: StubL1ViewState,
            }
        }
    }

    // Minimal stub account state for testing
    #[derive(Clone, Debug)]
    struct StubAccountState;

    #[derive(Clone, Debug)]
    struct StubSnarkAccountState;

    impl strata_ledger_types::ISnarkAccountState for StubSnarkAccountState {
        fn verifier_key(&self) -> &strata_predicate::PredicateKey {
            unimplemented!()
        }

        fn seqno(&self) -> u64 {
            0
        }

        fn next_inbox_idx(&self) -> u64 {
            0
        }

        fn inner_state_root(&self) -> strata_acct_types::Hash {
            [0u8; 32]
        }

        fn set_proof_state_directly(
            &mut self,
            _state: strata_acct_types::Hash,
            _next_inbox_idx: u64,
            _seqno: u64,
        ) {
        }

        fn update_inner_state(
            &mut self,
            _state: strata_acct_types::Hash,
            _seqno: u64,
            _extra_data: &[u8],
        ) -> strata_acct_types::AcctResult<()> {
            Ok(())
        }

        fn inbox_mmr(&self) -> &strata_acct_types::Mmr64 {
            unimplemented!()
        }

        fn insert_inbox_message(
            &mut self,
            _entry: strata_snark_acct_types::MessageEntry,
        ) -> strata_acct_types::AcctResult<()> {
            Ok(())
        }
    }

    impl strata_ledger_types::IAccountState for StubAccountState {
        type SnarkAccountState = StubSnarkAccountState;

        fn serial(&self) -> strata_acct_types::AccountSerial {
            0.into()
        }

        fn balance(&self) -> strata_acct_types::BitcoinAmount {
            0.into()
        }

        fn add_balance(&mut self, _coin: strata_ledger_types::Coin) {}

        fn take_balance(
            &mut self,
            _amt: strata_acct_types::BitcoinAmount,
        ) -> strata_acct_types::AcctResult<strata_ledger_types::Coin> {
            unimplemented!()
        }

        fn raw_ty(&self) -> strata_acct_types::AcctResult<strata_acct_types::RawAccountTypeId> {
            Ok(0)
        }

        fn ty(&self) -> strata_acct_types::AcctResult<strata_acct_types::AccountTypeId> {
            Ok(strata_acct_types::AccountTypeId::Empty)
        }

        fn get_type_state(
            &self,
        ) -> strata_acct_types::AcctResult<strata_ledger_types::AccountTypeState<Self>> {
            Ok(strata_ledger_types::AccountTypeState::Empty)
        }

        fn get_type_state_mut(
            &mut self,
        ) -> strata_acct_types::AcctResult<&mut strata_ledger_types::AccountTypeState<Self>> {
            unimplemented!()
        }

        fn set_type_state(
            &mut self,
            _state: strata_ledger_types::AccountTypeState<Self>,
        ) -> strata_acct_types::AcctResult<()> {
            unimplemented!()
        }
    }

    impl StateAccessor for MinimalStateAccessor {
        type GlobalState = MockGlobalState;
        type L1ViewState = StubL1ViewState;
        type AccountState = StubAccountState;

        fn global(&self) -> &Self::GlobalState {
            &self.global
        }

        fn global_mut(&mut self) -> &mut Self::GlobalState {
            &mut self.global
        }

        fn l1_view(&self) -> &Self::L1ViewState {
            &self.l1_view
        }

        fn l1_view_mut(&mut self) -> &mut Self::L1ViewState {
            &mut self.l1_view
        }

        fn check_account_exists(
            &self,
            _id: strata_acct_types::AccountId,
        ) -> strata_acct_types::AcctResult<bool> {
            unimplemented!()
        }

        fn get_account_id_from_serial(
            &self,
            _serial: strata_acct_types::AccountSerial,
        ) -> strata_acct_types::AcctResult<Option<strata_acct_types::AccountId>> {
            unimplemented!()
        }

        fn get_account_state(
            &self,
            _id: strata_acct_types::AccountId,
        ) -> strata_acct_types::AcctResult<Option<&Self::AccountState>> {
            unimplemented!()
        }

        fn get_account_state_mut(
            &mut self,
            _id: strata_acct_types::AccountId,
        ) -> strata_acct_types::AcctResult<Option<&mut Self::AccountState>> {
            unimplemented!()
        }

        fn update_account_state(
            &mut self,
            _id: strata_acct_types::AccountId,
            _state: Self::AccountState,
        ) -> strata_acct_types::AcctResult<()> {
            unimplemented!()
        }

        fn create_new_account(
            &mut self,
            _id: strata_acct_types::AccountId,
            _state: strata_ledger_types::AccountTypeState<Self::AccountState>,
        ) -> strata_acct_types::AcctResult<strata_acct_types::AccountSerial> {
            unimplemented!()
        }

        fn compute_state_root(&self) -> strata_primitives::Buf32 {
            unimplemented!()
        }
    }

    #[test]
    fn test_valid_tx_extra_no_constraints() {
        let state = MinimalStateAccessor::with_slot(100);
        let extra = TransactionExtra::default();

        let result = validate_tx_extra(&state, &extra);
        assert!(result.is_ok());
    }

    #[test]
    fn test_valid_tx_extra_within_range() {
        let state = MinimalStateAccessor::with_slot(100);
        let extra = TransactionExtra::new(Some(50), Some(150));

        let result = validate_tx_extra(&state, &extra);
        assert!(result.is_ok());
    }

    #[test]
    fn test_invalid_tx_extra_min_slot_too_high() {
        let state = MinimalStateAccessor::with_slot(100);

        // min_slot is 150, current is 100
        let extra = TransactionExtra::new(Some(150), None);

        let result = validate_tx_extra(&state, &extra);
        assert!(result.is_err());
        match result.unwrap_err() {
            StfError::InvalidTxExtra => {}
            err => panic!("Expected InvalidTxExtra, got {:?}", err),
        }
    }

    #[test]
    fn test_invalid_tx_extra_max_slot_too_low() {
        let state = MinimalStateAccessor::with_slot(100);

        // max_slot is 50, current is 100
        let extra = TransactionExtra::new(None, Some(50));

        let result = validate_tx_extra(&state, &extra);
        assert!(result.is_err());
        match result.unwrap_err() {
            StfError::InvalidTxExtra => {}
            err => panic!("Expected InvalidTxExtra, got {:?}", err),
        }
    }

    #[test]
    fn test_valid_tx_extra_boundary_min() {
        let state = MinimalStateAccessor::with_slot(100);

        // min_slot equals current slot
        let extra = TransactionExtra::new(Some(100), None);

        let result = validate_tx_extra(&state, &extra);
        assert!(result.is_ok());
    }

    #[test]
    fn test_valid_tx_extra_boundary_max() {
        let state = MinimalStateAccessor::with_slot(100);

        // max_slot equals current slot
        let extra = TransactionExtra::new(None, Some(100));

        let result = validate_tx_extra(&state, &extra);
        assert!(result.is_ok());
    }
}
