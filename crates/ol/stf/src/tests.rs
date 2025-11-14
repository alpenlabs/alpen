//! Tests for OL STF block assembly composition

use std::collections::HashMap;

use bitcoin::absolute;
use sha2::{Digest, Sha256};
use strata_acct_types::{
    AccountId, AccountSerial, AccountTypeId, AcctError, AcctResult, BitcoinAmount, Hash, Mmr64,
    RawAccountTypeId,
};
use strata_asm_common::AsmManifest;
use strata_btc_types::GenesisL1View;
use strata_ledger_types::{
    AccountTypeState, Coin, IAccountState, IGlobalState, IL1ViewState, ISnarkAccountState,
    StateAccessor,
};
use strata_ol_chain_types_new::{L1Update, OLBlockHeader};
use strata_params::{OperatorConfig, ProofPublishMode, RollupParams};
use strata_predicate::{PredicateKey, PredicateTypeId};
use strata_primitives::{Buf32, CredRule, Epoch, EpochCommitment, L1BlockCommitment, L1BlockId};
use strata_snark_acct_types::MessageEntry;

use super::*;
use crate::BlockExecContext;

#[derive(Clone, Debug)]
struct MockSnarkAccountState {
    verifier_key: PredicateKey,
    seqno: u64,
    next_inbox_idx: u64,
    inner_state_root: Hash,
    inbox_mmr: Mmr64,
}

impl ISnarkAccountState for MockSnarkAccountState {
    fn verifier_key(&self) -> &PredicateKey {
        &self.verifier_key
    }

    fn seqno(&self) -> u64 {
        self.seqno
    }

    fn next_inbox_idx(&self) -> u64 {
        self.next_inbox_idx
    }

    fn inner_state_root(&self) -> Hash {
        self.inner_state_root
    }

    fn set_proof_state_directly(&mut self, state: Hash, next_inbox_idx: u64, seqno: u64) {
        self.inner_state_root = state;
        self.next_inbox_idx = next_inbox_idx;
        self.seqno = seqno;
    }

    fn update_inner_state(
        &mut self,
        state: Hash,
        seqno: u64,
        _extra_data: &[u8],
    ) -> AcctResult<()> {
        self.inner_state_root = state;
        self.seqno = seqno;
        Ok(())
    }

    fn inbox_mmr(&self) -> &Mmr64 {
        &self.inbox_mmr
    }

    fn insert_inbox_message(&mut self, _entry: MessageEntry) -> AcctResult<()> {
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct MockAccountState {
    serial: AccountSerial,
    balance: BitcoinAmount,
    type_state: AccountTypeState<Self>,
}

impl IAccountState for MockAccountState {
    type SnarkAccountState = MockSnarkAccountState;

    fn serial(&self) -> AccountSerial {
        self.serial
    }

    fn balance(&self) -> BitcoinAmount {
        self.balance
    }

    fn add_balance(&mut self, coin: Coin) {
        self.balance = self
            .balance
            .checked_add(coin.amt())
            .expect("balance overflow");
        coin.safely_consume_unchecked();
    }

    fn take_balance(&mut self, amt: BitcoinAmount) -> AcctResult<Coin> {
        if self.balance < amt {
            return Err(AcctError::InsufficientBalance {
                requested: amt,
                available: self.balance,
            });
        }
        self.balance = self.balance.checked_sub(amt).unwrap();
        Ok(Coin::new_unchecked(amt))
    }

    fn raw_ty(&self) -> AcctResult<RawAccountTypeId> {
        Ok(match &self.type_state {
            AccountTypeState::Empty => 0,
            AccountTypeState::Snark(_) => 1,
        })
    }

    fn ty(&self) -> AcctResult<AccountTypeId> {
        Ok(match &self.type_state {
            AccountTypeState::Empty => AccountTypeId::Empty,
            AccountTypeState::Snark(_) => AccountTypeId::Snark,
        })
    }

    fn get_type_state(&self) -> AcctResult<AccountTypeState<Self>> {
        Ok(self.type_state.clone())
    }

    fn get_type_state_mut(&mut self) -> AcctResult<&mut AccountTypeState<Self>> {
        Ok(&mut self.type_state)
    }

    fn set_type_state(&mut self, state: AccountTypeState<Self>) -> AcctResult<()> {
        self.type_state = state;
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
struct MockGlobalState {
    cur_slot: u64,
}

impl IGlobalState for MockGlobalState {
    fn cur_slot(&self) -> u64 {
        self.cur_slot
    }

    fn set_cur_slot(&mut self, slot: u64) {
        self.cur_slot = slot
    }
}

#[derive(Clone, Debug)]
struct MockL1ViewState {
    cur_epoch: Epoch,
    last_l1_blkid: L1BlockId,
    last_l1_height: u32,
    asm_manifests_mmr: Mmr64,
    asm_recorded_epoch: EpochCommitment,
    total_ledger_balance: BitcoinAmount,
}

impl Default for MockL1ViewState {
    fn default() -> Self {
        Self {
            asm_manifests_mmr: Mmr64::new(32),
            asm_recorded_epoch: EpochCommitment::new(0, 0, Buf32::zero().into()),
            total_ledger_balance: 0.into(),
            cur_epoch: Default::default(),
            last_l1_blkid: Default::default(),
            last_l1_height: Default::default(),
        }
    }
}

impl IL1ViewState for MockL1ViewState {
    fn cur_epoch(&self) -> Epoch {
        self.cur_epoch
    }

    fn set_cur_epoch(&mut self, epoch: Epoch) {
        self.cur_epoch = epoch;
    }

    fn last_l1_blkid(&self) -> &L1BlockId {
        &self.last_l1_blkid
    }

    fn set_last_l1_blkid(&mut self, blkid: L1BlockId) {
        self.last_l1_blkid = blkid;
    }

    fn last_l1_height(&self) -> u32 {
        self.last_l1_height
    }

    fn set_last_l1_height(&mut self, height: u32) {
        // Cast away const for mock
        self.last_l1_height = height;
    }

    fn append_manifest(&mut self, _mf: AsmManifest) {
        // Stub for testing
    }

    fn asm_manifests_mmr(&self) -> &Mmr64 {
        &self.asm_manifests_mmr
    }

    fn asm_recorded_epoch(&self) -> &EpochCommitment {
        &self.asm_recorded_epoch
    }

    fn set_asm_recorded_epoch(&mut self, epoch: EpochCommitment) {
        self.asm_recorded_epoch = epoch;
    }

    fn total_ledger_balance(&self) -> BitcoinAmount {
        self.total_ledger_balance
    }

    fn increment_total_ledger_balance(&mut self, amt: BitcoinAmount) -> BitcoinAmount {
        self.total_ledger_balance = self.total_ledger_balance.checked_add(amt).unwrap();
        self.total_ledger_balance
    }

    fn decrement_total_ledger_balance(&mut self, amt: BitcoinAmount) -> BitcoinAmount {
        self.total_ledger_balance = self.total_ledger_balance.checked_sub(amt).unwrap();
        self.total_ledger_balance
    }
}

#[derive(Clone)]
struct MockStateAccessor {
    global: MockGlobalState,
    l1_view: MockL1ViewState,
    accounts: HashMap<AccountId, MockAccountState>,
    state_version: u64,
    next_serial: AccountSerial,
    serial_acct_id_map: HashMap<AccountSerial, AccountId>,
}

impl MockStateAccessor {
    fn set_cur_slot(&mut self, cur_slot: u64) {
        self.global.cur_slot = cur_slot;
    }
}

impl Default for MockStateAccessor {
    fn default() -> Self {
        Self {
            next_serial: 0.into(),
            global: Default::default(),
            l1_view: Default::default(),
            accounts: Default::default(),
            state_version: 1, // Start at 1 so initial state has non-zero root
            serial_acct_id_map: Default::default(),
        }
    }
}

impl StateAccessor for MockStateAccessor {
    type GlobalState = MockGlobalState;
    type L1ViewState = MockL1ViewState;
    type AccountState = MockAccountState;

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

    fn check_account_exists(&self, id: AccountId) -> AcctResult<bool> {
        Ok(self.accounts.contains_key(&id))
    }

    fn get_account_id_from_serial(&self, serial: AccountSerial) -> AcctResult<Option<AccountId>> {
        Ok(self.serial_acct_id_map.get(&serial).copied())
    }

    fn get_account_state(&self, id: AccountId) -> AcctResult<Option<&Self::AccountState>> {
        Ok(self.accounts.get(&id))
    }

    fn get_account_state_mut(
        &mut self,
        id: AccountId,
    ) -> AcctResult<Option<&mut Self::AccountState>> {
        Ok(self.accounts.get_mut(&id))
    }

    fn update_account_state(&mut self, id: AccountId, state: Self::AccountState) -> AcctResult<()> {
        if !self.accounts.contains_key(&id) {
            return Err(AcctError::NonExistentAccount(id));
        }
        self.accounts.insert(id, state);
        self.state_version += 1;
        Ok(())
    }

    fn create_new_account(
        &mut self,
        id: AccountId,
        state: AccountTypeState<Self::AccountState>,
    ) -> AcctResult<AccountSerial> {
        let serial = self.next_serial;
        let account = MockAccountState {
            serial,
            balance: BitcoinAmount::zero(),
            type_state: state,
        };
        self.accounts.insert(id, account);
        self.serial_acct_id_map.insert(self.next_serial, id);
        let ser: u32 = self.next_serial.into();
        self.next_serial = (ser + 1).into();

        Ok(serial)
    }

    fn compute_state_root(&self) -> Buf32 {
        // Hash actual account data for realistic state root calculation
        let mut hasher = Sha256::new();

        // Hash global state
        hasher.update(self.global.cur_slot.to_le_bytes());

        // Hash L1 view state
        hasher.update(self.l1_view.cur_epoch.to_le_bytes());
        hasher.update(self.l1_view.last_l1_height.to_le_bytes());
        // Hash L1 block ID (Buf32 implements AsRef<[u8; 32]>)
        let l1_blkid: Buf32 = self.l1_view.last_l1_blkid.into();
        hasher.update(l1_blkid.as_ref() as &[u8]);
        hasher.update(self.l1_view.total_ledger_balance.to_sat().to_le_bytes());

        // Hash all accounts in sorted order (by AccountId for determinism)
        let mut sorted_accounts: Vec<_> = self.accounts.iter().collect();
        sorted_accounts.sort_by_key(|(id, _)| *id);

        for (account_id, account) in sorted_accounts {
            // Hash account ID
            hasher.update(account_id.inner());

            // Hash serial
            let serial: u32 = account.serial.into();
            hasher.update(serial.to_le_bytes());

            // Hash balance
            hasher.update(account.balance.to_sat().to_le_bytes());

            // Hash type state
            match &account.type_state {
                AccountTypeState::Empty => {
                    hasher.update(&[0u8]); // Type discriminant
                }
                AccountTypeState::Snark(snark_state) => {
                    hasher.update(&[1u8]); // Type discriminant
                    hasher.update(snark_state.seqno.to_le_bytes());
                    hasher.update(snark_state.next_inbox_idx.to_le_bytes());
                    hasher.update(&snark_state.inner_state_root);
                    // Note: Not hashing verifier_key or inbox_mmr for simplicity in tests
                }
            }
        }

        let hash_result: [u8; 32] = hasher.finalize().into();
        Buf32::from(hash_result)
    }
}

/// Creates a test block header
fn create_test_header(slot: u64, epoch: Epoch, timestamp: u64, state_root: Buf32) -> OLBlockHeader {
    OLBlockHeader::new(
        timestamp,
        slot,
        epoch,
        Buf32::zero(), // parent_blkid
        Buf32::zero(), // body_root
        Buf32::zero(), // logs_root
        state_root,
    )
}

/// Creates test rollup params
fn test_rollup_params() -> RollupParams {
    RollupParams {
        magic_bytes: *b"TEST",
        block_time: 1000,
        da_tag: "da".to_string(),
        checkpoint_tag: "ckpt".to_string(),
        cred_rule: CredRule::Unchecked,
        genesis_l1_view: GenesisL1View {
            blk: L1BlockCommitment::new(absolute::Height::ZERO, L1BlockId::from(Buf32::zero())),
            next_target: 0x1d00ffff,
            epoch_start_timestamp: 0,
            last_11_timestamps: [0; 11],
        },
        operator_config: OperatorConfig::Static(vec![]),
        evm_genesis_block_hash: Buf32::zero(),
        evm_genesis_block_state_root: Buf32::zero(),
        l1_reorg_safe_depth: 6,
        target_l2_batch_size: 100,
        max_address_length: 64,
        deposit_amount: bitcoin::Amount::from_sat(1000),
        checkpoint_predicate: PredicateKey::new(PredicateTypeId::AlwaysAccept, vec![]),
        dispatch_assignment_dur: 100,
        proof_publish_mode: ProofPublishMode::Timeout(300),
        max_deposits_in_block: 10,
        network: bitcoin::Network::Regtest,
    }
}

/// These tests intend to demonstrate how block assembly can work with the stf design.
#[cfg(test)]
mod block_asm_tests {
    use strata_ol_chain_types_new::{
        OLBlock, OLBlockBody, OLTxSegment, SignedOLBlockHeader, compute_logs_root,
    };
    use strata_primitives::Buf64;

    use super::*;

    /// Composes a block from manually executed primitives.
    /// Returns the constructed block for further inspection or validation.
    fn compose_block(
        initial_state: &MockStateAccessor,
        prev_header: &OLBlockHeader,
        txs: Vec<strata_ol_chain_types_new::OLTransaction>,
        l1_update: Option<L1Update>,
        logs: Vec<strata_ol_chain_types_new::OLLog>,
        expected_state_root: Buf32,
    ) -> OLBlock {
        let logs_root = compute_logs_root(&logs);
        let body = OLBlockBody::new(OLTxSegment::new(txs), l1_update);
        let header = OLBlockHeader::new(
            0, // timestamp
            initial_state.global().cur_slot() + 1,
            initial_state.l1_view().cur_epoch(),
            prev_header.compute_root(),
            body.compute_root(),
            logs_root,
            expected_state_root,
        );
        let signed_header = SignedOLBlockHeader::new(header, Buf64::zero());
        OLBlock::new(signed_header, body)
    }

    /// Validates a block by executing it on fresh state via execute_block.
    /// This proves that the manually composed block is valid.
    fn validate_block(
        initial_state: &mut MockStateAccessor,
        prev_header: &OLBlockHeader,
        params: &RollupParams,
        block: OLBlock,
    ) {
        let ctx = BlockExecContext::new(prev_header.clone(), params.clone());
        execute_block(ctx, initial_state, block).expect("composed block should pass validation");
    }

    /// Tests terminal block (epoch sealing) composition and validation.
    ///
    /// Proves that:
    /// 1. execute_transactions and seal_epoch can be composed manually
    /// 2. Manual composition produces expected state changes (epoch increments, state root changes)
    /// 3. The composed result validates via execute_block (proves composition ≡ execute_block)
    #[test]
    fn test_terminal_block_assembly_composition() {
        // Setup mock state and context
        let cur_slot = 63;
        let mut state = MockStateAccessor::default();
        state.set_cur_slot(cur_slot);
        let mut initial_state = state.clone();
        let initial_root = initial_state.compute_state_root();

        let prev_header = create_test_header(cur_slot, 0, 0, initial_root);
        let params = test_rollup_params();
        let ctx = BlockExecContext::new(prev_header.clone(), params.clone());

        // 1. Execute transactions phase (empty for now - just testing structure)
        let txs = vec![]; // No transactions for simplicity
        // NOTE: In actual practice this would be a wrapper that returns valid and invalid txs
        execute_transactions(&ctx, &mut state, &txs).expect("transaction execution should succeed");

        // 2. Capture pre-seal state root
        let pre_seal_root = state.compute_state_root();
        let initial_epoch = state.l1_view().cur_epoch();
        let initial_l1_height = state.l1_view().last_l1_height();

        // 3. Create L1Update with pre-seal root
        let manifests = vec![]; // Empty manifests for simplicity
        let l1_update = L1Update::new(pre_seal_root, manifests);

        // 4. Seal epoch phase (independent from transaction execution)
        seal_epoch(&ctx, &mut state, &l1_update).expect("epoch sealing should succeed");

        // 5. Verify composition worked correctly
        let post_seal_root = state.compute_state_root();

        // State changed after epoch sealing (even with no manifests, epoch increment changes state)
        assert_ne!(
            pre_seal_root, post_seal_root,
            "state root should change after epoch sealing"
        );

        // Epoch incremented
        assert_eq!(
            state.l1_view().cur_epoch(),
            initial_epoch + 1,
            "epoch should increment after sealing"
        );

        // L1 height should be unchanged (no manifests processed)
        assert_eq!(
            state.l1_view().last_l1_height(),
            initial_l1_height,
            "L1 height should stay same with no manifests"
        );

        // 6. Compose block from manual execution
        let logs = ctx.into_logs();
        let block = compose_block(
            &initial_state,
            &prev_header,
            txs,
            Some(l1_update),
            logs,
            post_seal_root,
        );

        // 7. Validate composed block passes execute_block
        validate_block(&mut initial_state, &prev_header, &params, block);
    }

    /// Tests non-terminal block (no epoch sealing) composition and validation.
    ///
    /// Proves that:
    /// 1. execute_transactions works without epoch sealing
    /// 2. Epoch remains unchanged for non-terminal blocks
    /// 3. The composed result validates via execute_block
    #[test]
    fn test_non_terminal_block_execution() {
        let mut state = MockStateAccessor::default();
        state.set_cur_slot(10);
        let mut initial_state = state.clone();
        let initial_root = initial_state.compute_state_root();
        let prev_header = create_test_header(10, 0, 0, initial_root); // Not a terminal block
        let params = test_rollup_params();
        let ctx = BlockExecContext::new(prev_header.clone(), params.clone());

        let txs = vec![]; // Empty transactions
        let initial_epoch = state.l1_view().cur_epoch();

        execute_transactions(&ctx, &mut state, &txs).expect("should execute without epoch sealing");

        let root = state.compute_state_root();
        let epoch = state.l1_view().cur_epoch();

        // Can get state root without sealing
        assert_ne!(root, Buf32::zero());

        // Epoch unchanged
        assert_eq!(
            epoch, initial_epoch,
            "epoch should not change for non-terminal block"
        );

        // Compose and validate non-terminal block
        let logs = ctx.into_logs();
        let block = compose_block(&initial_state, &prev_header, txs, None, logs, root);
        validate_block(&mut initial_state, &prev_header, &params, block);
    }
}

/// Tests for transaction execution logic
#[cfg(test)]
mod transaction_execution_tests {
    use strata_ol_chain_types_new::{LogData, OLTransaction, TransactionExtra, TransactionPayload};
    use strata_snark_acct_types::{
        LedgerRefProofs, OutputTransfer, ProofState, SnarkAccountUpdate,
        SnarkAccountUpdateContainer, UpdateAccumulatorProofs, UpdateOperationData, UpdateOutputs,
    };

    use super::*;

    fn create_mock_snark_state() -> MockSnarkAccountState {
        MockSnarkAccountState {
            verifier_key: PredicateKey::new(PredicateTypeId::AlwaysAccept, vec![]),
            seqno: 0,
            next_inbox_idx: 0,
            inner_state_root: [0u8; 32],
            inbox_mmr: Mmr64::new(32),
        }
    }

    fn create_account_with_balance(
        state: &mut MockStateAccessor,
        id_byte: u8,
        balance: u64,
        is_snark: bool,
    ) -> AccountId {
        let account_id = AccountId::from([id_byte; 32]);
        let type_state = if is_snark {
            AccountTypeState::Snark(create_mock_snark_state())
        } else {
            AccountTypeState::Empty
        };

        state
            .create_new_account(account_id, type_state)
            .expect("account creation should succeed");

        // Set balance
        if balance > 0 {
            let coin = Coin::new_unchecked(BitcoinAmount::from_sat(balance));
            state
                .get_account_state_mut(account_id)
                .unwrap()
                .unwrap()
                .add_balance(coin);
        }

        account_id
    }

    fn get_snark_state(state: &MockStateAccessor, account_id: AccountId) -> MockSnarkAccountState {
        let type_state = state
            .get_account_state(account_id)
            .unwrap()
            .unwrap()
            .get_type_state()
            .unwrap();
        match type_state {
            AccountTypeState::Snark(s) => s,
            _ => panic!("Expected snark account"),
        }
    }

    fn get_updated_snark_state(
        state: &MockStateAccessor,
        account_id: AccountId,
    ) -> MockSnarkAccountState {
        let account = state.get_account_state(account_id).unwrap().unwrap();
        let type_state = account.get_type_state().unwrap();
        match type_state {
            AccountTypeState::Snark(s) => s,
            _ => panic!("Account should still be snark type after update"),
        }
    }

    fn assert_account_balance(
        state: &MockStateAccessor,
        account_id: AccountId,
        expected_sat: u64,
        msg: &str,
    ) {
        assert_eq!(
            state
                .get_account_state(account_id)
                .unwrap()
                .unwrap()
                .balance(),
            BitcoinAmount::from_sat(expected_sat),
            "{}",
            msg
        );
    }

    fn assert_single_log_emitted(
        ctx: BlockExecContext,
        expected_account: AccountId,
    ) -> strata_ol_chain_types_new::OLLog {
        let logs = ctx.into_logs();
        assert_eq!(logs.len(), 1, "Should emit exactly one log");
        let log = logs.into_iter().next().unwrap();
        assert_eq!(
            log.account_id(),
            expected_account,
            "Log should have correct account ID"
        );
        log
    }

    /// Creates a valid snark update that matches the current state of the account.
    /// This update will pass verification because:
    /// - seq_no matches the account's current seqno (required for verification)
    /// - next_inbox_idx progression is correct (no messages processed)
    /// - Uses AlwaysAccept predicate which passes witness verification
    /// - No outputs, so no balance checks needed
    fn create_valid_snark_update(
        snark_state: &MockSnarkAccountState,
    ) -> SnarkAccountUpdateContainer {
        let seqno = snark_state.seqno() + 1;
        let cur_inbox_idx = snark_state.next_inbox_idx();
        let messages = vec![];

        let new_inbox_idx = cur_inbox_idx + messages.len() as u64; // No messages processed
        let new_inner_state = [1u8; 32]; // Arbitrary new state

        let proof_state = ProofState::new(new_inner_state, new_inbox_idx);
        let ledger_refs = strata_snark_acct_types::LedgerRefs::new_empty();
        let outputs = UpdateOutputs::new_empty();
        let extra_data = vec![];

        let operation = UpdateOperationData::new(
            seqno,
            proof_state,
            messages,
            ledger_refs,
            outputs,
            extra_data,
        );

        let base = SnarkAccountUpdate::new(operation, vec![]);
        let accumulator_proofs = UpdateAccumulatorProofs::new(vec![], LedgerRefProofs::new(vec![]));

        SnarkAccountUpdateContainer::new(base, accumulator_proofs)
    }

    /// Creates a valid snark update with outputs (transfers/messages).
    /// Caller must ensure the account has sufficient balance for total_sent.
    fn create_valid_snark_update_with_outputs(
        snark_state: &MockSnarkAccountState,
        outputs: UpdateOutputs,
    ) -> SnarkAccountUpdateContainer {
        let seq_no = snark_state.seqno() + 1;
        let cur_inbox_idx = snark_state.next_inbox_idx();

        let new_inbox_idx = cur_inbox_idx;
        let new_inner_state = [2u8; 32];

        let proof_state = ProofState::new(new_inner_state, new_inbox_idx);
        let ledger_refs = strata_snark_acct_types::LedgerRefs::new_empty();

        let operation =
            UpdateOperationData::new(seq_no, proof_state, vec![], ledger_refs, outputs, vec![]);

        let base = SnarkAccountUpdate::new(operation, vec![]);
        let accumulator_proofs = UpdateAccumulatorProofs::new(vec![], LedgerRefProofs::new(vec![]));

        SnarkAccountUpdateContainer::new(base, accumulator_proofs)
    }

    fn create_mock_snark_update() -> SnarkAccountUpdateContainer {
        let proof_state = ProofState::new([0u8; 32], 0);
        let ledger_refs = strata_snark_acct_types::LedgerRefs::new_empty();
        let outputs = UpdateOutputs::new_empty();

        let operation = UpdateOperationData::new(
            1,           // seq_no
            proof_state, // proof_state
            vec![],      // messages
            ledger_refs, // ledger_refs
            outputs,     // outputs
            vec![],      // extra_data
        );
        let base = SnarkAccountUpdate::new(operation, vec![]);
        let accumulator_proofs = UpdateAccumulatorProofs::new(vec![], LedgerRefProofs::new(vec![]));

        SnarkAccountUpdateContainer::new(base, accumulator_proofs)
    }

    #[test]
    fn test_tx_on_nonexistent_account_fails() {
        let mut state = MockStateAccessor::default();
        state.set_cur_slot(10);
        let prev_header = create_test_header(10, 0, 0, state.compute_state_root());
        let params = test_rollup_params();
        let ctx = BlockExecContext::new(prev_header, params);

        let nonexistent = AccountId::from([99u8; 32]);
        let update = create_mock_snark_update();
        let tx = OLTransaction::new(
            TransactionPayload::SnarkAccountUpdate {
                target: nonexistent,
                update,
            },
            TransactionExtra::default(),
        );

        let result = execute_transaction(&ctx, &mut state, &tx);

        assert!(result.is_err());
        match result.unwrap_err() {
            StfError::Account(AcctError::NonExistentAccount(id)) => {
                assert_eq!(id, nonexistent);
            }
            err => panic!("Expected NonExistentAccount error, got {:?}", err),
        }
    }

    #[test]
    fn test_snark_update_on_non_snark_account_fails() {
        let mut state = MockStateAccessor::default();
        state.set_cur_slot(10);

        // Create empty (non-snark) account
        let account_id = create_account_with_balance(&mut state, 1, 1000, false);

        let prev_header = create_test_header(10, 0, 0, state.compute_state_root());
        let params = test_rollup_params();
        let ctx = BlockExecContext::new(prev_header, params);

        let update = create_mock_snark_update();
        let tx = OLTransaction::new(
            TransactionPayload::SnarkAccountUpdate {
                target: account_id,
                update,
            },
            TransactionExtra::default(),
        );

        let result = execute_transaction(&ctx, &mut state, &tx);

        assert!(result.is_err());
        match result.unwrap_err() {
            StfError::SnarkUpdateForNonSnarkAccount(id) => {
                assert_eq!(id, account_id);
            }
            err => panic!(
                "Expected SnarkUpdateForNonSnarkAccount error, got {:?}",
                err
            ),
        }
    }

    #[test]
    fn test_generic_account_message_unsupported() {
        let mut state = MockStateAccessor::default();
        state.set_cur_slot(10);
        let account_id = create_account_with_balance(&mut state, 1, 1000, true);

        let prev_header = create_test_header(10, 0, 0, state.compute_state_root());
        let params = test_rollup_params();
        let ctx = BlockExecContext::new(prev_header, params);

        let tx = OLTransaction::new(
            TransactionPayload::GenericAccountMessage {
                target: account_id,
                payload: vec![1, 2, 3],
            },
            TransactionExtra::default(),
        );

        let result = execute_transaction(&ctx, &mut state, &tx);

        assert!(result.is_err());
        match result.unwrap_err() {
            StfError::UnsupportedTransaction => {}
            err => panic!("Expected UnsupportedTransaction error, got {:?}", err),
        }
    }

    #[test]
    fn test_successful_snark_update_no_outputs() {
        let mut state = MockStateAccessor::default();
        state.set_cur_slot(10);

        // Create snark account with some balance
        let account_id = create_account_with_balance(&mut state, 1, 5000, true);
        let initial_balance = 5000;
        let initial_state_root = state.compute_state_root();

        // Get the snark state to create a matching update
        let snark_state = get_snark_state(&state, account_id);

        let initial_inner_state = snark_state.inner_state_root();
        let initial_seqno = snark_state.seqno();
        let initial_inbox_idx = snark_state.next_inbox_idx();

        // Create valid update with no outputs
        let update = create_valid_snark_update(&snark_state);

        let prev_header = create_test_header(10, 0, 0, initial_state_root);
        let params = test_rollup_params();
        let ctx = BlockExecContext::new(prev_header, params);

        let tx = OLTransaction::new(
            TransactionPayload::SnarkAccountUpdate {
                target: account_id,
                update,
            },
            TransactionExtra::default(),
        );

        // Execute transaction
        let result = execute_transaction(&ctx, &mut state, &tx);
        assert!(result.is_ok(), "Transaction should succeed: {:?}", result);

        // Verify account state changes
        assert_account_balance(
            &state,
            account_id,
            initial_balance,
            "Balance should not change without outputs",
        );

        // Check snark state was updated
        let updated_snark = get_updated_snark_state(&state, account_id);

        // Inner state should have changed
        assert_ne!(
            updated_snark.inner_state_root(),
            initial_inner_state,
            "Inner state should change"
        );
        assert_eq!(
            updated_snark.inner_state_root(),
            [1u8; 32],
            "Inner state should match update"
        );

        // Seqno should remain the same (as per current semantics)
        assert_eq!(
            updated_snark.seqno(),
            initial_seqno + 1,
            "Seqno should increment"
        );

        // Inbox idx should remain same (no messages processed)
        assert_eq!(
            updated_snark.next_inbox_idx(),
            initial_inbox_idx,
            "Inbox idx should stay same with no messages"
        );

        // State root should change
        let new_state_root = state.compute_state_root();
        assert_ne!(
            new_state_root, initial_state_root,
            "State root should change after update"
        );

        // Verify log was emitted
        let log = assert_single_log_emitted(ctx, account_id);

        match log.log_data() {
            LogData::SnarkAccountUpdate(log_data) => {
                assert_eq!(
                    log_data.to_msg_idx(),
                    initial_inbox_idx,
                    "Log should have correct inbox idx"
                );
                assert_eq!(
                    log_data.new_proof_state(),
                    [1u8; 32].into(),
                    "Log should have correct new state"
                );
            }
            _ => panic!("Expected SnarkAccountUpdate log"),
        }
    }

    #[test]
    fn test_successful_snark_update_with_balance_deduction() {
        let mut state = MockStateAccessor::default();
        state.set_cur_slot(10);

        // Create two accounts: sender (snark) and receiver (empty)
        let init_sender_bal = 10000;
        let sender_id = create_account_with_balance(&mut state, 1, init_sender_bal, true);
        let receiver_id = create_account_with_balance(&mut state, 2, 0, false);

        let transfer_amount = 3000u64;

        // Get the snark state
        let snark_state = get_snark_state(&state, sender_id);

        // Create update with a transfer output
        let transfer = OutputTransfer::new(receiver_id, BitcoinAmount::from_sat(transfer_amount));
        let outputs = UpdateOutputs::new(vec![transfer], vec![]);
        let update = create_valid_snark_update_with_outputs(&snark_state, outputs);

        let prev_header = create_test_header(10, 0, 0, state.compute_state_root());
        let params = test_rollup_params();
        let ctx = BlockExecContext::new(prev_header, params);

        let tx = OLTransaction::new(
            TransactionPayload::SnarkAccountUpdate {
                target: sender_id,
                update,
            },
            TransactionExtra::default(),
        );

        // Execute transaction
        let result = execute_transaction(&ctx, &mut state, &tx);
        assert!(result.is_ok(), "Transaction should succeed: {:?}", result);

        // Verify sender balance was deducted
        let expected_sender_balance = init_sender_bal - transfer_amount;
        assert_account_balance(
            &state,
            sender_id,
            expected_sender_balance,
            "Sender balance should be deducted",
        );

        // Verify receiver balance increased
        assert_account_balance(
            &state,
            receiver_id,
            transfer_amount,
            "Receiver should receive the transfer",
        );

        // Verify inner state changed
        let updated_snark = get_updated_snark_state(&state, sender_id);
        assert_eq!(
            updated_snark.inner_state_root(),
            [2u8; 32],
            "Inner state should match update"
        );

        // Verify log was emitted
        assert_single_log_emitted(ctx, sender_id);
    }

    #[test]
    fn test_snark_update_insufficient_balance_fails() {
        let mut state = MockStateAccessor::default();
        state.set_cur_slot(10);

        // Create sender with insufficient balance
        let sender_id = create_account_with_balance(&mut state, 1, 1000, true);
        let receiver_id = create_account_with_balance(&mut state, 2, 0, false);

        let snark_state = get_snark_state(&state, sender_id);

        // Try to transfer more than available balance
        let transfer = OutputTransfer::new(receiver_id, BitcoinAmount::from_sat(5000));
        let outputs = UpdateOutputs::new(vec![transfer], vec![]);
        let update = create_valid_snark_update_with_outputs(&snark_state, outputs);

        let prev_header = create_test_header(10, 0, 0, state.compute_state_root());
        let params = test_rollup_params();
        let ctx = BlockExecContext::new(prev_header, params);

        let tx = OLTransaction::new(
            TransactionPayload::SnarkAccountUpdate {
                target: sender_id,
                update,
            },
            TransactionExtra::default(),
        );

        // Should fail during verification (before balance deduction)
        let result = execute_transaction(&ctx, &mut state, &tx);
        assert!(result.is_err(), "Should fail with insufficient balance");

        match result.unwrap_err() {
            StfError::Account(AcctError::InsufficientBalance {
                requested,
                available,
            }) => {
                assert_eq!(requested, BitcoinAmount::from_sat(5000));
                assert_eq!(available, BitcoinAmount::from_sat(1000));
            }
            err => panic!("Expected InsufficientBalance error, got {:?}", err),
        }

        // Verify balances unchanged
        assert_account_balance(
            &state,
            sender_id,
            1000,
            "Sender balance should be unchanged",
        );
        assert_account_balance(
            &state,
            receiver_id,
            0,
            "Receiver balance should be unchanged",
        );
    }
}
