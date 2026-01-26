//! Integration tests for combined state layers.
//!
//! These tests verify that multiple wrapper layers can be composed together
//! and work correctly.

use std::collections::{BTreeMap, VecDeque};

use strata_acct_types::{
    AccountId, AccountTypeId, AcctError, BitcoinAmount, Hash, Mmr64, MsgPayload,
};
use strata_asm_manifest_types::AsmManifest;
use strata_da_framework::decode_buf_exact;
use strata_identifiers::{AccountSerial, Buf32, EpochCommitment, L1BlockId, L1Height, WtxidsRoot};
use strata_ledger_types::{
    AccountTypeState, AccountTypeStateRef, Coin, IAccountState, IAccountStateMut,
    ISnarkAccountState, ISnarkAccountStateMut, IStateAccessor, NewAccountData,
};
use strata_merkle::CompactMmr64;
use strata_ol_da::{AccountTypeInit, MAX_MSG_PAYLOAD_BYTES, OLDaPayloadV1};
use strata_ol_state_types::{OLSnarkAccountState, OLState, WriteBatch};
use strata_predicate::{MAX_CONDITION_LEN, PredicateKey, PredicateTypeId};
use strata_snark_acct_types::{MessageEntry, Seqno};

use crate::{
    BatchDiffState, DaAccumulatingState, DaAccumulationError, IndexerState, WriteTrackingState,
    test_utils::*,
};

// =============================================================================
// IndexerState over WriteTrackingState tests
// =============================================================================

/// Test that IndexerState can wrap WriteTrackingState and both function correctly.
#[test]
fn test_indexer_over_write_tracking_basic() {
    let account_id = test_account_id(1);
    let (base_state, _serial) =
        setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1_000));

    // Create the layer stack: IndexerState<WriteTrackingState<&OLState>>
    let batch = WriteBatch::new_from_state(&base_state);
    let tracking = WriteTrackingState::new(&base_state, batch);
    let indexer = IndexerState::new(tracking);

    // Verify we can read through both layers
    let account = indexer.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(account.balance(), BitcoinAmount::from_sat(1_000));
}

/// Test inbox message tracking through both layers.
#[test]
fn test_combined_inbox_message_tracking() {
    let account_id = test_account_id(1);
    let (base_state, _serial) =
        setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1_000));

    let batch = WriteBatch::new_from_state(&base_state);
    let tracking = WriteTrackingState::new(&base_state, batch);
    let mut indexer = IndexerState::new(tracking);

    // Insert an inbox message through the combined stack
    let msg = test_message_entry(50, 0, 2_000);
    indexer
        .update_account(account_id, |acct| {
            acct.as_snark_account_mut()
                .unwrap()
                .insert_inbox_message(msg.clone())
        })
        .unwrap()
        .unwrap();

    // Extract the layers
    let (tracking, indexer_writes) = indexer.into_parts();
    let batch = tracking.into_batch();

    // Verify IndexerState captured the inbox write
    assert_eq!(indexer_writes.inbox_messages().len(), 1);
    assert_eq!(indexer_writes.inbox_messages()[0].account_id, account_id);
    assert_eq!(indexer_writes.inbox_messages()[0].index, 0);

    // Verify WriteTrackingState has the modified account in the batch
    assert!(batch.ledger().contains_account(&account_id));

    // Verify base state is unchanged
    let base_account = base_state.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(
        base_account
            .as_snark_account()
            .unwrap()
            .inbox_mmr()
            .num_entries(),
        0
    );
}

/// Test manifest tracking through combined layers.
#[test]
fn test_combined_manifest_tracking() {
    let base_state = OLState::new_genesis();
    let batch = WriteBatch::new_from_state(&base_state);
    let tracking = WriteTrackingState::new(&base_state, batch);
    let mut indexer = IndexerState::new(tracking);

    // Append a manifest through the combined stack
    let height = L1Height::from(100u32);
    let l1_blkid = L1BlockId::from(Buf32::from([1u8; 32]));
    let wtxids_root = WtxidsRoot::from(Buf32::from([2u8; 32]));
    let manifest = AsmManifest::new(height.into(), l1_blkid, wtxids_root, vec![]);

    indexer.append_manifest(height, manifest);

    // Verify IndexerState captured the manifest write
    let (_, indexer_writes) = indexer.into_parts();
    assert_eq!(indexer_writes.manifests().len(), 1);
    assert_eq!(indexer_writes.manifests()[0].height, height);
}

/// Test balance modifications through combined layers.
#[test]
fn test_combined_balance_modification() {
    let account_id = test_account_id(1);
    let (base_state, _serial) =
        setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1_000));

    let batch = WriteBatch::new_from_state(&base_state);
    let tracking = WriteTrackingState::new(&base_state, batch);
    let mut indexer = IndexerState::new(tracking);

    // Modify balance through the combined stack
    indexer
        .update_account(account_id, |acct| {
            let coin = Coin::new_unchecked(BitcoinAmount::from_sat(500));
            acct.add_balance(coin);
        })
        .unwrap();

    // Extract and verify
    let (tracking, _) = indexer.into_parts();
    let batch = tracking.into_batch();

    // Verify the account is in the batch with updated balance
    let batch_account = batch.ledger().get_account(&account_id).unwrap();
    assert_eq!(batch_account.balance(), BitcoinAmount::from_sat(1_500));

    // Verify base state is unchanged
    let base_account = base_state.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(base_account.balance(), BitcoinAmount::from_sat(1_000));
}

/// Test account creation through combined layers.
#[test]
fn test_combined_account_creation() {
    let base_state = OLState::new_genesis();
    let batch = WriteBatch::new_from_state(&base_state);
    let tracking = WriteTrackingState::new(&base_state, batch);
    let mut indexer = IndexerState::new(tracking);

    // Create a new account through the combined stack
    let account_id = test_account_id(1);
    let snark_state = test_snark_account_state(1);
    let new_acct = NewAccountData::new(
        BitcoinAmount::from_sat(5_000),
        AccountTypeState::Snark(snark_state),
    );

    let serial = indexer.create_new_account(account_id, new_acct).unwrap();

    // Verify the account exists through the stack
    assert!(indexer.check_account_exists(account_id).unwrap());
    let account = indexer.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(account.serial(), serial);
    assert_eq!(account.balance(), BitcoinAmount::from_sat(5_000));

    // Extract and verify it's in the batch
    let (tracking, _) = indexer.into_parts();
    let batch = tracking.into_batch();
    assert!(batch.ledger().contains_account(&account_id));
}

/// Test global state modifications through combined layers.
#[test]
fn test_combined_global_state_modification() {
    let base_state = OLState::new_genesis();
    let batch = WriteBatch::new_from_state(&base_state);
    let tracking = WriteTrackingState::new(&base_state, batch);
    let mut indexer = IndexerState::new(tracking);

    // Modify slot through the combined stack
    indexer.set_cur_slot(42);
    assert_eq!(indexer.cur_slot(), 42);

    // Modify epoch
    indexer.set_cur_epoch(5);
    assert_eq!(indexer.cur_epoch(), 5);

    // Extract and verify
    let (tracking, _) = indexer.into_parts();
    let batch = tracking.into_batch();

    assert_eq!(batch.global().get_cur_slot(), 42);
    assert_eq!(batch.epochal().cur_epoch(), 5);
}

/// Test multiple operations through combined layers.
#[test]
fn test_combined_multiple_operations() {
    let account_id_1 = test_account_id(1);
    let account_id_2 = test_account_id(2);

    // Setup base state with one account
    let (base_state, _) =
        setup_state_with_snark_account(account_id_1, 1, BitcoinAmount::from_sat(1_000));

    let batch = WriteBatch::new_from_state(&base_state);
    let tracking = WriteTrackingState::new(&base_state, batch);
    let mut indexer = IndexerState::new(tracking);

    // Create a new account
    let snark_state_2 = test_snark_account_state(2);
    let new_acct = NewAccountData::new(
        BitcoinAmount::from_sat(2_000),
        AccountTypeState::Snark(snark_state_2),
    );
    indexer.create_new_account(account_id_2, new_acct).unwrap();

    // Insert messages to both accounts
    let msg1 = test_message_entry(10, 0, 1_000);
    indexer
        .update_account(account_id_1, |acct| {
            acct.as_snark_account_mut()
                .unwrap()
                .insert_inbox_message(msg1.clone())
        })
        .unwrap()
        .unwrap();

    let msg2 = test_message_entry(20, 0, 2_000);
    indexer
        .update_account(account_id_2, |acct| {
            acct.as_snark_account_mut()
                .unwrap()
                .insert_inbox_message(msg2.clone())
        })
        .unwrap()
        .unwrap();

    // Modify slot
    indexer.set_cur_slot(100);

    // Extract and verify all changes
    let (tracking, indexer_writes) = indexer.into_parts();
    let batch = tracking.into_batch();

    // Verify IndexerState tracked both inbox writes
    assert_eq!(indexer_writes.inbox_messages().len(), 2);

    // Verify batch has both accounts
    assert!(batch.ledger().contains_account(&account_id_1));
    assert!(batch.ledger().contains_account(&account_id_2));

    // Verify slot was updated
    assert_eq!(batch.global().get_cur_slot(), 100);
}

// =============================================================================
// WriteTrackingState over BatchDiffState tests
// =============================================================================

/// Test that WriteTrackingState can wrap BatchDiffState and all write operations work correctly.
/// This verifies that we can build on top of a read-only diff layer with pending batches.
#[test]
fn test_write_tracking_over_batch_diff_basic() {
    let account_id = test_account_id(1);
    let (base_state, _serial) =
        setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));

    // Create a pending batch with some modifications
    let mut pending_batch = WriteBatch::new_from_state(&base_state);
    pending_batch.global_mut().set_cur_slot(50);
    pending_batch.epochal_mut().set_cur_epoch(3);

    // Create BatchDiffState with the pending batch
    let pending_batches = vec![pending_batch];
    let diff_state = BatchDiffState::new(&base_state, &pending_batches);

    // Create WriteTrackingState on top of BatchDiffState
    // The write batch needs to be initialized with values from the diff state
    // (WriteTrackingState reads global/epochal from its own batch, not from base)
    let mut write_batch = WriteBatch::new_from_state(&base_state);
    write_batch.global_mut().set_cur_slot(diff_state.cur_slot());
    write_batch
        .epochal_mut()
        .set_cur_epoch(diff_state.cur_epoch());
    let tracking = WriteTrackingState::new(&diff_state, write_batch);

    // Verify we can read through the layers (account from base via diff_state)
    let account = tracking.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(account.balance(), BitcoinAmount::from_sat(1000));

    // Global/epochal come from the write batch (which we initialized from diff_state)
    assert_eq!(tracking.cur_slot(), 50);
    assert_eq!(tracking.cur_epoch(), 3);
}

/// Test that update_account works through WriteTrackingState over BatchDiffState.
#[test]
fn test_write_tracking_over_batch_diff_update_account() {
    let account_id = test_account_id(1);
    let (base_state, _serial) =
        setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));

    // Create BatchDiffState (empty batches = pure passthrough)
    let pending_batches: Vec<WriteBatch<_>> = vec![];
    let diff_state = BatchDiffState::new(&base_state, &pending_batches);

    // Create WriteTrackingState on top
    let write_batch = WriteBatch::new_from_state(&base_state);
    let mut tracking = WriteTrackingState::new(&diff_state, write_batch);

    // Update account balance
    tracking
        .update_account(account_id, |acct| {
            let coin = Coin::new_unchecked(BitcoinAmount::from_sat(500));
            acct.add_balance(coin);
        })
        .unwrap();

    // Verify the update worked
    let account = tracking.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(account.balance(), BitcoinAmount::from_sat(1500));

    // Verify it's in the write batch
    let batch = tracking.into_batch();
    assert!(batch.ledger().contains_account(&account_id));
    assert_eq!(
        batch.ledger().get_account(&account_id).unwrap().balance(),
        BitcoinAmount::from_sat(1500)
    );
}

/// Test that create_new_account works through WriteTrackingState over BatchDiffState.
#[test]
fn test_write_tracking_over_batch_diff_create_account() {
    let base_state = OLState::new_genesis();

    // Create BatchDiffState with empty batches
    let pending_batches: Vec<WriteBatch<_>> = vec![];
    let diff_state = BatchDiffState::new(&base_state, &pending_batches);

    // Create WriteTrackingState on top
    let write_batch = WriteBatch::new_from_state(&base_state);
    let mut tracking = WriteTrackingState::new(&diff_state, write_batch);

    // Create a new account
    let account_id = test_account_id(1);
    let snark_state = test_snark_account_state(1);
    let new_acct = NewAccountData::new(
        BitcoinAmount::from_sat(5000),
        AccountTypeState::Snark(snark_state),
    );
    let serial = tracking.create_new_account(account_id, new_acct).unwrap();

    // Verify the account exists
    assert!(tracking.check_account_exists(account_id).unwrap());
    let account = tracking.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(account.serial(), serial);
    assert_eq!(account.balance(), BitcoinAmount::from_sat(5000));

    // Verify it's in the write batch
    let batch = tracking.into_batch();
    assert!(batch.ledger().contains_account(&account_id));
}

/// Test that global/epochal setters work through WriteTrackingState over BatchDiffState.
#[test]
fn test_write_tracking_over_batch_diff_global_epochal_setters() {
    let base_state = OLState::new_genesis();

    // Create BatchDiffState with a pending batch that has slot=50, epoch=3
    let mut pending_batch = WriteBatch::new_from_state(&base_state);
    pending_batch.global_mut().set_cur_slot(50);
    pending_batch.epochal_mut().set_cur_epoch(3);
    let pending_batches = vec![pending_batch];
    let diff_state = BatchDiffState::new(&base_state, &pending_batches);

    // Create WriteTrackingState on top
    let write_batch = WriteBatch::new_from_state(&base_state);
    let mut tracking = WriteTrackingState::new(&diff_state, write_batch);

    // Modify slot and epoch through WriteTrackingState
    tracking.set_cur_slot(100);
    tracking.set_cur_epoch(10);

    // Verify the values are updated
    assert_eq!(tracking.cur_slot(), 100);
    assert_eq!(tracking.cur_epoch(), 10);

    // Verify they're in the write batch
    let batch = tracking.into_batch();
    assert_eq!(batch.global().get_cur_slot(), 100);
    assert_eq!(batch.epochal().cur_epoch(), 10);
}

/// Test that inbox message insertion works through WriteTrackingState over BatchDiffState.
#[test]
fn test_write_tracking_over_batch_diff_inbox_message() {
    let account_id = test_account_id(1);
    let (base_state, _serial) =
        setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1_000));

    // Create BatchDiffState with empty batches
    let pending_batches: Vec<WriteBatch<_>> = vec![];
    let diff_state = BatchDiffState::new(&base_state, &pending_batches);

    // Create WriteTrackingState on top
    let write_batch = WriteBatch::new_from_state(&base_state);
    let mut tracking = WriteTrackingState::new(&diff_state, write_batch);

    // Insert an inbox message
    let msg = test_message_entry(50, 0, 2000);
    tracking
        .update_account(account_id, |acct| {
            acct.as_snark_account_mut()
                .unwrap()
                .insert_inbox_message(msg.clone())
        })
        .unwrap()
        .unwrap();

    // Verify the message was inserted
    let account = tracking.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(
        account
            .as_snark_account()
            .unwrap()
            .inbox_mmr()
            .num_entries(),
        1
    );

    // Verify base is unchanged
    let base_account = base_state.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(
        base_account
            .as_snark_account()
            .unwrap()
            .inbox_mmr()
            .num_entries(),
        0
    );
}

// =============================================================================
// DaAccumulatingState tests
// =============================================================================

fn build_simple_blob() -> Vec<u8> {
    let account_id = test_account_id(1);
    let (mut state, _) =
        setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
    let source_account_id = test_account_id(7);
    state
        .create_new_account(
            source_account_id,
            NewAccountData::new(
                BitcoinAmount::from_sat(0),
                AccountTypeState::Snark(test_snark_account_state(2)),
            ),
        )
        .unwrap();
    let mut da_state = DaAccumulatingState::new(state);

    da_state.set_cur_slot(10);

    let msg = test_message_entry(7, 0, 2000);
    da_state
        .update_account(account_id, |acct| {
            let coin = Coin::new_unchecked(BitcoinAmount::from_sat(500));
            acct.add_balance(coin);
            acct.as_snark_account_mut()
                .unwrap()
                .insert_inbox_message(msg.clone())
        })
        .unwrap()
        .unwrap();

    da_state
        .take_completed_epoch_da_blob()
        .expect("build DA blob")
        .expect("expected DA blob")
}

#[derive(Clone, Debug)]
struct TestSnarkState {
    update_vk: PredicateKey,
    inner_state_root: Hash,
    seqno: Seqno,
    inbox_mmr: Mmr64,
}

impl TestSnarkState {
    fn new(update_vk: Vec<u8>) -> Self {
        let generic_mmr = CompactMmr64::<[u8; 32]>::new(64);
        let inbox_mmr = Mmr64::from_generic(&generic_mmr);
        let update_vk = PredicateKey::new(PredicateTypeId::AlwaysAccept, update_vk);
        Self {
            update_vk,
            inner_state_root: Hash::from([0u8; 32]),
            seqno: Seqno::zero(),
            inbox_mmr,
        }
    }
}

impl ISnarkAccountState for TestSnarkState {
    fn update_vk(&self) -> &PredicateKey {
        &self.update_vk
    }

    fn seqno(&self) -> Seqno {
        self.seqno
    }

    fn inner_state_root(&self) -> Hash {
        self.inner_state_root
    }

    fn inbox_mmr(&self) -> &Mmr64 {
        &self.inbox_mmr
    }

    fn next_inbox_msg_idx(&self) -> u64 {
        0
    }
}

impl ISnarkAccountStateMut for TestSnarkState {
    fn set_proof_state_directly(&mut self, state: Hash, _next_read_idx: u64, seqno: Seqno) {
        self.inner_state_root = state;
        self.seqno = seqno;
    }

    fn update_inner_state(
        &mut self,
        inner_state: Hash,
        next_read_idx: u64,
        seqno: Seqno,
        _extra_data: &[u8],
    ) -> strata_acct_types::AcctResult<()> {
        self.set_proof_state_directly(inner_state, next_read_idx, seqno);
        Ok(())
    }

    fn insert_inbox_message(&mut self, _entry: MessageEntry) -> strata_acct_types::AcctResult<()> {
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct TestAccountState {
    serial: AccountSerial,
    balance: BitcoinAmount,
    ty: AccountTypeId,
    snark: Option<TestSnarkState>,
}

impl IAccountState for TestAccountState {
    type SnarkAccountState = TestSnarkState;

    fn serial(&self) -> AccountSerial {
        self.serial
    }

    fn balance(&self) -> BitcoinAmount {
        self.balance
    }

    fn ty(&self) -> AccountTypeId {
        self.ty
    }

    fn type_state(&self) -> AccountTypeStateRef<'_, Self> {
        match self.snark.as_ref() {
            Some(snark) => AccountTypeStateRef::Snark(snark),
            None => AccountTypeStateRef::Empty,
        }
    }

    fn as_snark_account(&self) -> strata_acct_types::AcctResult<&Self::SnarkAccountState> {
        self.snark
            .as_ref()
            .ok_or(AcctError::MismatchedType(self.ty, AccountTypeId::Snark))
    }
}

impl IAccountStateMut for TestAccountState {
    type SnarkAccountStateMut = TestSnarkState;

    fn add_balance(&mut self, coin: Coin) {
        let new_balance = self.balance.to_sat() + coin.amt().to_sat();
        self.balance = BitcoinAmount::from_sat(new_balance);
        coin.safely_consume_unchecked();
    }

    fn take_balance(&mut self, _amt: BitcoinAmount) -> strata_acct_types::AcctResult<Coin> {
        Err(AcctError::Unsupported)
    }

    fn as_snark_account_mut(
        &mut self,
    ) -> strata_acct_types::AcctResult<&mut Self::SnarkAccountStateMut> {
        self.snark
            .as_mut()
            .ok_or(AcctError::MismatchedType(self.ty, AccountTypeId::Snark))
    }
}

#[derive(Debug)]
struct TestState {
    accounts: BTreeMap<AccountId, TestAccountState>,
    next_serial: AccountSerial,
    serial_overrides: VecDeque<AccountSerial>,
    cur_slot: u64,
    cur_epoch: u32,
    last_l1_blkid: L1BlockId,
    last_l1_height: L1Height,
    asm_recorded_epoch: EpochCommitment,
    total_ledger_balance: BitcoinAmount,
}

impl TestState {
    fn new_with_serials(serials: Vec<AccountSerial>) -> Self {
        Self {
            accounts: BTreeMap::new(),
            next_serial: AccountSerial::one(),
            serial_overrides: VecDeque::from(serials),
            cur_slot: 0,
            cur_epoch: 0,
            last_l1_blkid: L1BlockId::from(Buf32::zero()),
            last_l1_height: L1Height::from(0u32),
            asm_recorded_epoch: EpochCommitment::null(),
            total_ledger_balance: BitcoinAmount::ZERO,
        }
    }
}

impl IStateAccessor for TestState {
    type AccountState = TestAccountState;
    type AccountStateMut = TestAccountState;

    fn cur_slot(&self) -> u64 {
        self.cur_slot
    }

    fn set_cur_slot(&mut self, slot: u64) {
        self.cur_slot = slot;
    }

    fn cur_epoch(&self) -> u32 {
        self.cur_epoch
    }

    fn set_cur_epoch(&mut self, epoch: u32) {
        self.cur_epoch = epoch;
    }

    fn last_l1_blkid(&self) -> &L1BlockId {
        &self.last_l1_blkid
    }

    fn last_l1_height(&self) -> L1Height {
        self.last_l1_height
    }

    fn append_manifest(&mut self, _height: L1Height, _mf: strata_asm_manifest_types::AsmManifest) {}

    fn asm_recorded_epoch(&self) -> &EpochCommitment {
        &self.asm_recorded_epoch
    }

    fn set_asm_recorded_epoch(&mut self, epoch: EpochCommitment) {
        self.asm_recorded_epoch = epoch;
    }

    fn total_ledger_balance(&self) -> BitcoinAmount {
        self.total_ledger_balance
    }

    fn set_total_ledger_balance(&mut self, amt: BitcoinAmount) {
        self.total_ledger_balance = amt;
    }

    fn check_account_exists(&self, id: AccountId) -> strata_acct_types::AcctResult<bool> {
        Ok(self.accounts.contains_key(&id))
    }

    fn get_account_state(
        &self,
        id: AccountId,
    ) -> strata_acct_types::AcctResult<Option<&Self::AccountState>> {
        Ok(self.accounts.get(&id))
    }

    fn update_account<R, F>(&mut self, id: AccountId, f: F) -> strata_acct_types::AcctResult<R>
    where
        F: FnOnce(&mut Self::AccountStateMut) -> R,
    {
        let acct = self
            .accounts
            .get_mut(&id)
            .ok_or(AcctError::UpdateNonexistentAccount(id))?;
        Ok(f(acct))
    }

    fn create_new_account(
        &mut self,
        id: AccountId,
        new_acct_data: NewAccountData<Self::AccountState>,
    ) -> strata_acct_types::AcctResult<AccountSerial> {
        if self.accounts.contains_key(&id) {
            return Err(AcctError::CreateExistingAccount(id));
        }

        let serial = if let Some(serial) = self.serial_overrides.pop_front() {
            serial
        } else {
            let serial = self.next_serial;
            self.next_serial = self.next_serial.incr();
            serial
        };

        let balance = new_acct_data.initial_balance();
        let (ty, snark) = match new_acct_data.into_type_state() {
            AccountTypeState::Empty => (AccountTypeId::Empty, None),
            AccountTypeState::Snark(snark_state) => (AccountTypeId::Snark, Some(snark_state)),
        };

        let acct = TestAccountState {
            serial,
            balance,
            ty,
            snark,
        };
        self.accounts.insert(id, acct);
        Ok(serial)
    }

    fn find_account_id_by_serial(
        &self,
        serial: AccountSerial,
    ) -> strata_acct_types::AcctResult<Option<AccountId>> {
        Ok(self
            .accounts
            .iter()
            .find_map(|(id, acct)| (acct.serial == serial).then_some(*id)))
    }

    fn next_account_serial(&self) -> AccountSerial {
        self.next_serial
    }

    fn compute_state_root(&self) -> strata_acct_types::AcctResult<Buf32> {
        Ok(Buf32::zero())
    }

    fn asm_manifests_mmr(&self) -> &Mmr64 {
        todo!()
    }
}

#[test]
fn test_da_blob_deterministic() {
    let blob1 = build_simple_blob();
    let blob2 = build_simple_blob();
    assert_eq!(blob1, blob2);
}

#[test]
fn test_epoch_sealing_suspends_da_tracking() {
    let account_id = test_account_id(1);
    let (state, _) = setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1_000));
    let mut da_state = DaAccumulatingState::new(state);

    da_state
        .update_account(account_id, |acct| {
            let coin = Coin::new_unchecked(BitcoinAmount::from_sat(50));
            acct.add_balance(coin);
        })
        .unwrap();

    da_state.begin_epoch_sealing();

    da_state
        .update_account(account_id, |acct| {
            let coin = Coin::new_unchecked(BitcoinAmount::from_sat(25));
            acct.add_balance(coin);
        })
        .unwrap();
    assert!(da_state.post_seal_writes_detected());

    let err = da_state
        .take_completed_epoch_da_blob()
        .expect_err("expected post-seal write error");
    assert!(matches!(
        err,
        DaAccumulationError::PostSealWrites {
            context: "take_completed_epoch_da_blob"
        }
    ));
}

#[test]
fn test_post_seal_writes_surface_on_epoch_transition() {
    let account_id = test_account_id(1);
    let (state, _) = setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1_000));
    let mut da_state = DaAccumulatingState::new(state);

    da_state.begin_epoch_sealing();
    da_state.set_cur_slot(da_state.cur_slot() + 1);
    assert!(da_state.post_seal_writes_detected());

    let next_epoch = da_state.cur_epoch() + 1;
    da_state.set_cur_epoch(next_epoch);

    let err = da_state
        .take_completed_epoch_da_blob()
        .expect_err("expected post-seal write error");
    assert!(matches!(
        err,
        DaAccumulationError::PostSealWrites {
            context: "set_cur_epoch"
        }
    ));
}

#[test]
fn test_account_diffs_ordered_by_serial() {
    let mut state = OLState::new_genesis();
    let account_id_1 = test_account_id(1);
    let account_id_2 = test_account_id(2);

    let snark_state_1 = test_snark_account_state(1);
    let snark_state_2 = test_snark_account_state(2);
    state
        .create_new_account(
            account_id_1,
            NewAccountData::new(
                BitcoinAmount::from_sat(1000),
                AccountTypeState::Snark(snark_state_1),
            ),
        )
        .unwrap();
    state
        .create_new_account(
            account_id_2,
            NewAccountData::new(
                BitcoinAmount::from_sat(2000),
                AccountTypeState::Snark(snark_state_2),
            ),
        )
        .unwrap();

    let mut da_state = DaAccumulatingState::new(state);

    // Update higher serial first, then lower serial.
    da_state
        .update_account(account_id_2, |acct| {
            let coin = Coin::new_unchecked(BitcoinAmount::from_sat(50));
            acct.add_balance(coin);
        })
        .unwrap();
    da_state
        .update_account(account_id_1, |acct| {
            let coin = Coin::new_unchecked(BitcoinAmount::from_sat(75));
            acct.add_balance(coin);
        })
        .unwrap();

    let blob_bytes = da_state
        .take_completed_epoch_da_blob()
        .expect("build DA blob")
        .expect("expected DA blob");
    let blob: OLDaPayloadV1 = decode_buf_exact(&blob_bytes).expect("decode DA blob");

    let diffs = blob.state_diff.ledger.account_diffs.entries();
    assert!(
        diffs
            .windows(2)
            .all(|w| w[0].account_serial <= w[1].account_serial)
    );
}

#[test]
fn test_new_account_post_state_encoded() {
    let mut da_state = DaAccumulatingState::new(TestState::new_with_serials(vec![]));
    let account_id = test_account_id(9);
    let update_vk = vec![7u8; 4];
    let snark_state = TestSnarkState::new(update_vk.clone());
    let new_acct = NewAccountData::new(
        BitcoinAmount::from_sat(100),
        AccountTypeState::Snark(snark_state),
    );
    da_state.create_new_account(account_id, new_acct).unwrap();

    da_state
        .update_account(account_id, |acct| {
            let coin = Coin::new_unchecked(BitcoinAmount::from_sat(50));
            acct.add_balance(coin);
            acct.as_snark_account_mut()
                .unwrap()
                .set_proof_state_directly(test_hash(9), 0, Seqno::new(1));
        })
        .unwrap();

    let blob_bytes = da_state
        .take_completed_epoch_da_blob()
        .expect("build DA blob")
        .expect("expected DA blob");
    let blob: OLDaPayloadV1 = decode_buf_exact(&blob_bytes).expect("decode DA blob");

    let new_accounts = blob.state_diff.ledger.new_accounts.entries();
    assert_eq!(new_accounts.len(), 1);
    let entry = &new_accounts[0];
    assert_eq!(entry.account_id, account_id);
    assert_eq!(entry.init.balance, BitcoinAmount::from_sat(150));
    match &entry.init.type_state {
        AccountTypeInit::Snark(init) => {
            assert_eq!(init.initial_state_root, test_hash(9));
            // The VK is stored with the predicate type ID prefix, so we need to compare
            // with the full predicate key bytes (type ID + raw VK bytes)
            let expected_vk = PredicateKey::new(PredicateTypeId::AlwaysAccept, update_vk.clone());
            assert_eq!(
                init.update_vk.as_slice(),
                expected_vk.as_buf_ref().to_bytes()
            );
        }
        _ => panic!("expected snark account init"),
    }
    let diffs = blob.state_diff.ledger.account_diffs.entries();
    assert!(diffs.is_empty());
}

#[test]
fn test_new_account_vk_persisted_from_ol_state() {
    let mut da_state = DaAccumulatingState::new(OLState::new_genesis());
    let account_id = test_account_id(10);
    let snark_state = OLSnarkAccountState::new_fresh(PredicateKey::always_accept(), test_hash(4));
    let new_acct = NewAccountData::new(
        BitcoinAmount::from_sat(100),
        AccountTypeState::Snark(snark_state.clone()),
    );
    da_state.create_new_account(account_id, new_acct).unwrap();

    let blob_bytes = da_state
        .take_completed_epoch_da_blob()
        .expect("build DA blob")
        .expect("expected DA blob");
    let blob: OLDaPayloadV1 = decode_buf_exact(&blob_bytes).expect("decode DA blob");

    let new_accounts = blob.state_diff.ledger.new_accounts.entries();
    assert_eq!(new_accounts.len(), 1);
    match &new_accounts[0].init.type_state {
        AccountTypeInit::Snark(init) => {
            assert_eq!(
                init.update_vk.as_slice(),
                snark_state.update_vk().as_buf_ref().to_bytes()
            );
        }
        _ => panic!("expected snark account init"),
    }
}

#[test]
fn test_take_resets_accumulator() {
    let account_id = test_account_id(1);
    let (state, _) = setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1000));
    let mut da_state = DaAccumulatingState::new(state);

    // Finalize once after making changes.
    da_state
        .update_account(account_id, |acct| {
            let coin = Coin::new_unchecked(BitcoinAmount::from_sat(123));
            acct.add_balance(coin);
        })
        .unwrap();
    da_state
        .take_completed_epoch_da_blob()
        .expect("build DA blob")
        .expect("expected DA blob");

    // Finalize again without any new changes.
    let blob_bytes = da_state
        .take_completed_epoch_da_blob()
        .expect("build DA blob")
        .expect("expected DA blob");
    let blob: OLDaPayloadV1 = decode_buf_exact(&blob_bytes).expect("decode DA blob");

    assert!(blob.state_diff.ledger.account_diffs.entries().is_empty());
}

#[test]
fn test_da_blob_size_limit() {
    // Test that the DA blob size limit is enforced by creating many accounts
    // with large VK data to exceed the limit.
    let mut test_state = TestState::new_with_serials(vec![]);
    test_state.next_serial = AccountSerial::one();

    let mut da_state = DaAccumulatingState::new(test_state);

    // Create many accounts with moderately sized VKs to approach the limit
    let vk_data = vec![0u8; 1024]; // 1KB VK per account
    for i in 0..=255 {
        let account_id = test_account_id(i);
        let snark_state = TestSnarkState::new(vk_data.clone());
        let new_acct = NewAccountData::new(
            BitcoinAmount::from_sat(0),
            AccountTypeState::Snark(snark_state),
        );
        if da_state.create_new_account(account_id, new_acct).is_err() {
            break;
        }
    }

    // Try to finalize - should fail with PayloadTooLarge
    let result = da_state.take_completed_epoch_da_blob();
    assert!(
        matches!(result, Err(DaAccumulationError::PayloadTooLarge { .. })),
        "expected DA blob size limit error"
    );
}

#[test]
fn test_vk_size_truncates_to_predicate_limit() {
    let mut da_state = DaAccumulatingState::new(TestState::new_with_serials(vec![]));
    let account_id = test_account_id(1);
    let oversized_vk_len = MAX_CONDITION_LEN as usize + 10;
    let snark_state = TestSnarkState::new(vec![0u8; oversized_vk_len]);
    let new_acct = NewAccountData::new(
        BitcoinAmount::from_sat(0),
        AccountTypeState::Snark(snark_state),
    );
    da_state.create_new_account(account_id, new_acct).unwrap();

    let blob_bytes = da_state
        .take_completed_epoch_da_blob()
        .expect("build DA blob")
        .expect("expected DA blob");
    let blob: OLDaPayloadV1 = decode_buf_exact(&blob_bytes).expect("decode DA blob");

    let new_accounts = blob.state_diff.ledger.new_accounts.entries();
    assert_eq!(new_accounts.len(), 1);
    match &new_accounts[0].init.type_state {
        AccountTypeInit::Snark(init) => {
            assert_eq!(
                init.update_vk.as_slice().len(),
                MAX_CONDITION_LEN as usize + 1
            );
        }
        _ => panic!("expected snark account init"),
    }
}

#[test]
fn test_message_source_missing_is_rejected() {
    let account_id = test_account_id(1);
    let (state, _) = setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1_000));
    let mut da_state = DaAccumulatingState::new(state);

    let payload = MsgPayload::new(BitcoinAmount::from_sat(0), vec![0u8; 4]);
    let missing_source = test_account_id(99);
    let msg = MessageEntry::new(missing_source, 0, payload);
    da_state
        .update_account(account_id, |acct| {
            acct.as_snark_account_mut()
                .unwrap()
                .insert_inbox_message(msg)
        })
        .unwrap()
        .unwrap();

    let result = da_state.take_completed_epoch_da_blob();
    assert!(matches!(
        result,
        Err(DaAccumulationError::MessageSourceMissing(id)) if id == missing_source
    ));
}

#[test]
fn test_message_payload_size_limit() {
    let account_id = test_account_id(1);
    let (state, _) = setup_state_with_snark_account(account_id, 1, BitcoinAmount::from_sat(1_000));
    let mut da_state = DaAccumulatingState::new(state);

    let payload = MsgPayload::new(
        BitcoinAmount::from_sat(0),
        vec![0u8; MAX_MSG_PAYLOAD_BYTES + 1],
    );
    let msg = MessageEntry::new(test_account_id(2), 0, payload);
    da_state
        .update_account(account_id, |acct| {
            acct.as_snark_account_mut()
                .unwrap()
                .insert_inbox_message(msg)
        })
        .unwrap()
        .unwrap();

    let result = da_state.take_completed_epoch_da_blob();
    assert!(matches!(
        result,
        Err(DaAccumulationError::MessagePayloadTooLarge { .. })
    ));
}

#[test]
fn test_early_serial_gap_detection() {
    let mut da_state = DaAccumulatingState::new(TestState::new_with_serials(vec![
        AccountSerial::new(1),
        AccountSerial::new(3),
    ]));
    let account_id_1 = test_account_id(1);
    let account_id_2 = test_account_id(2);
    let snark_state = TestSnarkState::new(vec![]);
    let new_acct = NewAccountData::new(
        BitcoinAmount::from_sat(0),
        AccountTypeState::Snark(snark_state),
    );
    da_state
        .create_new_account(account_id_1, new_acct.clone())
        .unwrap();
    da_state.create_new_account(account_id_2, new_acct).unwrap();

    let result = da_state.take_completed_epoch_da_blob();
    assert!(matches!(
        result,
        Err(DaAccumulationError::NewAccountSerialGap(_, _))
    ));
}

#[test]
fn test_expected_first_serial_mismatch() {
    let mut da_state =
        DaAccumulatingState::new(TestState::new_with_serials(vec![AccountSerial::new(5)]));
    let account_id = test_account_id(1);
    let snark_state = TestSnarkState::new(vec![]);
    let new_acct = NewAccountData::new(
        BitcoinAmount::from_sat(0),
        AccountTypeState::Snark(snark_state),
    );
    da_state.create_new_account(account_id, new_acct).unwrap();

    let result = da_state.take_completed_epoch_da_blob();
    assert!(matches!(
        result,
        Err(DaAccumulationError::NewAccountSerialGap(_, _))
    ));
}

/// Test reading account from pending batch through WriteTrackingState over BatchDiffState.
#[test]
fn test_write_tracking_over_batch_diff_reads_from_pending_batch() {
    let base_state = OLState::new_genesis();

    // Create a pending batch with a new account
    let account_id_in_batch = test_account_id(1);
    let mut pending_batch = WriteBatch::new_from_state(&base_state);
    let snark_state = test_snark_account_state(1);
    let new_acct = NewAccountData::new(
        BitcoinAmount::from_sat(3000),
        AccountTypeState::Snark(snark_state),
    );
    let serial = base_state.next_account_serial();
    pending_batch
        .ledger_mut()
        .create_account_from_data(account_id_in_batch, new_acct, serial);

    let pending_batches = vec![pending_batch];
    let diff_state = BatchDiffState::new(&base_state, &pending_batches);

    // Create WriteTrackingState on top
    let write_batch = WriteBatch::new_from_state(&base_state);
    let tracking = WriteTrackingState::new(&diff_state, write_batch);

    // Should be able to read the account from the pending batch
    assert!(tracking.check_account_exists(account_id_in_batch).unwrap());
    let account = tracking
        .get_account_state(account_id_in_batch)
        .unwrap()
        .unwrap();
    assert_eq!(account.balance(), BitcoinAmount::from_sat(3000));
}

/// Test that WriteTrackingState over BatchDiffState can update an account from the pending batch.
#[test]
fn test_write_tracking_over_batch_diff_update_account_from_pending_batch() {
    let base_state = OLState::new_genesis();

    // Create a pending batch with a new account
    let account_id = test_account_id(1);
    let mut pending_batch = WriteBatch::new_from_state(&base_state);
    let snark_state = test_snark_account_state(1);
    let new_acct = NewAccountData::new(
        BitcoinAmount::from_sat(3000),
        AccountTypeState::Snark(snark_state),
    );
    let serial = base_state.next_account_serial();
    pending_batch
        .ledger_mut()
        .create_account_from_data(account_id, new_acct, serial);

    let pending_batches = vec![pending_batch];
    let diff_state = BatchDiffState::new(&base_state, &pending_batches);

    // Create WriteTrackingState on top
    let write_batch = WriteBatch::new_from_state(&base_state);
    let mut tracking = WriteTrackingState::new(&diff_state, write_batch);

    // Update the account (copy-on-write from pending batch to write batch)
    tracking
        .update_account(account_id, |acct| {
            let coin = Coin::new_unchecked(BitcoinAmount::from_sat(500));
            acct.add_balance(coin);
        })
        .unwrap();

    // Verify the update worked
    let account = tracking.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(account.balance(), BitcoinAmount::from_sat(3500));

    // Verify it's now in the write batch with the updated balance
    let batch = tracking.into_batch();
    assert!(batch.ledger().contains_account(&account_id));
    assert_eq!(
        batch.ledger().get_account(&account_id).unwrap().balance(),
        BitcoinAmount::from_sat(3500)
    );
}

// =============================================================================
// Tests verifying layer isolation
// =============================================================================

/// Test that modifications through combined layers don't affect the base state.
#[test]
fn test_combined_layers_preserve_base_state() {
    let account_id = test_account_id(1);
    let initial_balance = BitcoinAmount::from_sat(1000);
    let (base_state, _) = setup_state_with_snark_account(account_id, 1, initial_balance);

    // Save original values
    let original_slot = base_state.cur_slot();
    let original_epoch = base_state.cur_epoch();
    let original_inbox_count = base_state
        .get_account_state(account_id)
        .unwrap()
        .unwrap()
        .as_snark_account()
        .unwrap()
        .inbox_mmr()
        .num_entries();

    let batch = WriteBatch::new_from_state(&base_state);
    let tracking = WriteTrackingState::new(&base_state, batch);
    let mut indexer = IndexerState::new(tracking);

    // Make various modifications
    indexer.set_cur_slot(999);
    indexer.set_cur_epoch(99);
    indexer
        .update_account(account_id, |acct| {
            let coin = Coin::new_unchecked(BitcoinAmount::from_sat(500));
            acct.add_balance(coin);
            acct.as_snark_account_mut()
                .unwrap()
                .insert_inbox_message(test_message_entry(1, 0, 1000))
                .unwrap();
        })
        .unwrap();

    // Discard the layers (don't apply to base)
    drop(indexer);

    // Verify base state is completely unchanged
    assert_eq!(base_state.cur_slot(), original_slot);
    assert_eq!(base_state.cur_epoch(), original_epoch);

    let account = base_state.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(account.balance(), initial_balance);
    assert_eq!(
        account
            .as_snark_account()
            .unwrap()
            .inbox_mmr()
            .num_entries(),
        original_inbox_count
    );
}
