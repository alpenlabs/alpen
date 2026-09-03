//! Integration tests for combined state layers.
//!
//! These tests verify that multiple wrapper layers can be composed together
//! and work correctly.

use std::collections::{BTreeMap, VecDeque};

use strata_acct_types::{
    AccountId, AccountTypeId, BitcoinAmount, Hash, L1BlockRecord, MessageEntry, Mmr64, MsgPayload,
};
use strata_da_framework::decode_buf_exact;
use strata_identifiers::{AccountSerial, Buf32, EpochCommitment, L1BlockId, L1Height};
use strata_merkle::CompactMmr64;
use strata_ol_da_types_v1::{AccountTypeInitV1, MAX_MSG_PAYLOAD_BYTES, OLDaPayloadV1};
use strata_ol_state_types::*;
use strata_ol_state_types_v1::{MAX_PENDING_ASM_LOGS, OLSnarkAccountStateV1, WriteBatch};
use strata_predicate::{MAX_CONDITION_LEN, PredicateKey, PredicateTypeId};
use strata_snark_acct_types::Seqno;

use crate::test_utils::*;
use crate::{
    BatchDiffState, DaAccumulatingState, DaAccumulationError, IndexerState, WriteTrackingState,
};

// =============================================================================
// IndexerState over WriteTrackingState tests
// =============================================================================

/// Test that IndexerState can wrap WriteTrackingState and both function correctly.
#[test]
fn test_indexer_over_write_tracking_basic() {
    let account_id = test_account_id(1);
    let (base_layer, _serial) = setup_layer_with_snark_account(
        account_id,
        1,
        BitcoinAmount::try_from(1_000).expect("amount must not exceed the Bitcoin money supply"),
    );

    // Create the layer stack: IndexerState<WriteTrackingState<&MemoryStateBaseLayer>>
    let tracking = WriteTrackingState::new_empty(&base_layer);
    let indexer = IndexerState::new(tracking);

    // Verify we can read through both layers
    let account = indexer.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(
        account.balance(),
        BitcoinAmount::try_from(1_000).expect("amount must not exceed the Bitcoin money supply")
    );
}

/// Test inbox message tracking through both layers.
#[test]
fn test_combined_inbox_message_tracking() {
    let account_id = test_account_id(1);
    let (base_layer, _serial) = setup_layer_with_snark_account(
        account_id,
        1,
        BitcoinAmount::try_from(1_000).expect("amount must not exceed the Bitcoin money supply"),
    );

    let tracking = WriteTrackingState::new_empty(&base_layer);
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
    let base_account = base_layer.get_account_state(account_id).unwrap().unwrap();
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
    let base_layer = create_test_base_layer();
    let tracking = WriteTrackingState::new_empty(&base_layer);
    let mut indexer = IndexerState::new(tracking);

    // Append an L1 block record through the combined stack
    let height = L1Height::from(100u32);
    let record = L1BlockRecord::new([1u8; 32], [2u8; 32]);

    indexer.append_l1_block_rec(height, record);

    // Verify IndexerState captured the record write
    let (_, indexer_writes) = indexer.into_parts();
    assert_eq!(indexer_writes.l1_block_records().len(), 1);
    assert_eq!(indexer_writes.l1_block_records()[0].height, height);
}

/// Test balance modifications through combined layers.
#[test]
fn test_combined_balance_modification() {
    let account_id = test_account_id(1);
    let (base_layer, _serial) = setup_layer_with_snark_account(
        account_id,
        1,
        BitcoinAmount::try_from(1_000).expect("amount must not exceed the Bitcoin money supply"),
    );

    let tracking = WriteTrackingState::new_empty(&base_layer);
    let mut indexer = IndexerState::new(tracking);

    // Modify balance through the combined stack
    indexer
        .update_account(account_id, |acct| {
            let coin = Coin::new_unchecked(
                BitcoinAmount::try_from(500)
                    .expect("amount must not exceed the Bitcoin money supply"),
            );
            acct.add_balance(coin);
        })
        .unwrap();

    // Extract and verify
    let (tracking, _) = indexer.into_parts();
    let batch = tracking.into_batch();

    // Verify the account is in the batch with updated balance
    let batch_account = batch.ledger().get_account(&account_id).unwrap();
    assert_eq!(
        batch_account.balance(),
        BitcoinAmount::try_from(1_500).expect("amount must not exceed the Bitcoin money supply")
    );

    // Verify base state is unchanged
    let base_account = base_layer.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(
        base_account.balance(),
        BitcoinAmount::try_from(1_000).expect("amount must not exceed the Bitcoin money supply")
    );
}

/// Test account creation through combined layers.
#[test]
fn test_combined_account_creation() {
    let base_layer = create_test_base_layer();
    let tracking = WriteTrackingState::new_empty(&base_layer);
    let mut indexer = IndexerState::new(tracking);

    // Create a new account through the combined stack
    let account_id = test_account_id(1);
    let new_acct = test_new_snark_account_data(
        &test_snark_account_state(1),
        BitcoinAmount::try_from(5_000).expect("amount must not exceed the Bitcoin money supply"),
    );

    let serial = indexer.create_new_account(account_id, new_acct).unwrap();

    // Verify the account exists through the stack
    assert!(indexer.check_account_exists(account_id).unwrap());
    let account = indexer.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(account.serial(), serial);
    assert_eq!(
        account.balance(),
        BitcoinAmount::try_from(5_000).expect("amount must not exceed the Bitcoin money supply")
    );

    // Extract and verify it's in the batch
    let (tracking, _) = indexer.into_parts();
    let batch = tracking.into_batch();
    assert!(batch.ledger().contains_account(&account_id));
}

/// Test global state modifications through combined layers.
#[test]
fn test_combined_global_state_modification() {
    let base_layer = create_test_base_layer();
    let tracking = WriteTrackingState::new_empty(&base_layer);
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

    assert_eq!(batch.global_writes().cur_slot, Some(42));
    assert_eq!(batch.epochal_writes().cur_epoch, Some(5));
}

/// Test multiple operations through combined layers.
#[test]
fn test_combined_multiple_operations() {
    let account_id_1 = test_account_id(1);
    let account_id_2 = test_account_id(2);

    // Setup base layer with one account
    let (base_layer, _) = setup_layer_with_snark_account(
        account_id_1,
        1,
        BitcoinAmount::try_from(1_000).expect("amount must not exceed the Bitcoin money supply"),
    );

    let tracking = WriteTrackingState::new_empty(&base_layer);
    let mut indexer = IndexerState::new(tracking);

    // Create a new account
    let new_acct = test_new_snark_account_data(
        &test_snark_account_state(2),
        BitcoinAmount::try_from(2_000).expect("amount must not exceed the Bitcoin money supply"),
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
    assert_eq!(batch.global_writes().cur_slot, Some(100));
}

// =============================================================================
// WriteTrackingState over BatchDiffState tests
//
// The behavior of this stack against a base with no pending writes is covered
// by the shared suite instantiated in `write_tracking_layer::tests`. What is
// left here is what that suite deliberately doesn't reach: reads and
// copy-on-write against a *non-empty* pending batch.
// =============================================================================

/// Reads through the stack resolve against a *non-empty* pending batch: an
/// account that exists only in the batch, and global/epochal fields the batch
/// overrides.
#[test]
fn test_write_tracking_over_batch_diff_reads_from_pending_batch() {
    let base_layer = create_test_base_layer();

    let account_id_in_batch = test_account_id(1);
    let mut pending_batch = WriteBatch::default();
    let new_acct = test_new_snark_account_data(
        &test_snark_account_state(1),
        BitcoinAmount::try_from(3000).expect("amount must not exceed the Bitcoin money supply"),
    );
    let serial = base_layer.next_account_serial();
    pending_batch
        .ledger_mut()
        .create_account_from_data(account_id_in_batch, new_acct, serial);
    pending_batch.global_writes_mut().cur_slot = Some(50);
    pending_batch.epochal_writes_mut().cur_epoch = Some(3);

    let pending_batches = vec![pending_batch];
    let diff_state = BatchDiffState::new(&base_layer, &pending_batches);
    let tracking = WriteTrackingState::new_empty(&diff_state);

    // The account exists only in the pending batch.
    assert!(tracking.check_account_exists(account_id_in_batch).unwrap());
    let account = tracking
        .get_account_state(account_id_in_batch)
        .unwrap()
        .unwrap();
    assert_eq!(
        account.balance(),
        BitcoinAmount::try_from(3000).expect("amount must not exceed the Bitcoin money supply")
    );

    // Global and epochal reads resolve to the pending batch, not the base.
    assert_eq!(tracking.cur_slot(), 50);
    assert_eq!(tracking.cur_epoch(), 3);
    assert_ne!(tracking.cur_slot(), base_layer.cur_slot());
    assert_ne!(tracking.cur_epoch(), base_layer.cur_epoch());
}

/// Writes through the stack take precedence over the values the pending batch
/// supplies.
#[test]
fn test_write_tracking_over_batch_diff_write_overrides_pending_batch() {
    let base_layer = create_test_base_layer();

    let mut pending_batch = WriteBatch::default();
    pending_batch.global_writes_mut().cur_slot = Some(50);
    pending_batch.epochal_writes_mut().cur_epoch = Some(3);
    let pending_batches = vec![pending_batch];
    let diff_state = BatchDiffState::new(&base_layer, &pending_batches);
    let mut tracking = WriteTrackingState::new_empty(&diff_state);

    tracking.set_cur_slot(100);
    tracking.set_cur_epoch(10);

    assert_eq!(tracking.cur_slot(), 100);
    assert_eq!(tracking.cur_epoch(), 10);

    let batch = tracking.into_batch();
    assert_eq!(batch.global_writes().cur_slot, Some(100));
    assert_eq!(batch.epochal_writes().cur_epoch, Some(10));
}

/// An account that exists only in the pending batch is copied into the write
/// batch when it is updated.
#[test]
fn test_write_tracking_over_batch_diff_update_account_from_pending_batch() {
    let base_layer = create_test_base_layer();

    let account_id = test_account_id(1);
    let mut pending_batch = WriteBatch::default();
    let new_acct = test_new_snark_account_data(
        &test_snark_account_state(1),
        BitcoinAmount::try_from(3000).expect("amount must not exceed the Bitcoin money supply"),
    );
    let serial = base_layer.next_account_serial();
    pending_batch
        .ledger_mut()
        .create_account_from_data(account_id, new_acct, serial);

    let pending_batches = vec![pending_batch];
    let diff_state = BatchDiffState::new(&base_layer, &pending_batches);
    let mut tracking = WriteTrackingState::new_empty(&diff_state);

    // Copy-on-write from the pending batch into this layer's write batch.
    tracking
        .update_account(account_id, |acct| {
            let coin = Coin::new_unchecked(
                BitcoinAmount::try_from(500)
                    .expect("amount must not exceed the Bitcoin money supply"),
            );
            acct.add_balance(coin);
        })
        .unwrap();

    let account = tracking.get_account_state(account_id).unwrap().unwrap();
    assert_eq!(
        account.balance(),
        BitcoinAmount::try_from(3500).expect("amount must not exceed the Bitcoin money supply")
    );

    let batch = tracking.into_batch();
    assert!(batch.ledger().contains_account(&account_id));
    assert_eq!(
        batch.ledger().get_account(&account_id).unwrap().balance(),
        BitcoinAmount::try_from(3500).expect("amount must not exceed the Bitcoin money supply")
    );
}

// =============================================================================
// DaAccumulatingState tests
// =============================================================================

fn build_simple_blob() -> Vec<u8> {
    let account_id = test_account_id(1);
    let (mut layer, _) = setup_layer_with_snark_account(
        account_id,
        1,
        BitcoinAmount::try_from(1000).expect("amount must not exceed the Bitcoin money supply"),
    );
    let source_account_id = test_account_id(7);
    layer
        .create_new_account(
            source_account_id,
            test_new_snark_account_data(
                &test_snark_account_state(2),
                BitcoinAmount::try_from(0)
                    .expect("amount must not exceed the Bitcoin money supply"),
            ),
        )
        .unwrap();
    let mut da_state = DaAccumulatingState::new(layer);

    da_state.set_cur_slot(10);

    let msg = test_message_entry(7, 0, 2000);
    da_state
        .update_account(account_id, |acct| {
            let coin = Coin::new_unchecked(
                BitcoinAmount::try_from(500)
                    .expect("amount must not exceed the Bitcoin money supply"),
            );
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
        let update_vk = PredicateKey::try_new(PredicateTypeId::AlwaysAccept, update_vk)
            .expect("predicate condition must fit within the maximum length");
        Self {
            update_vk,
            inner_state_root: Hash::from([0u8; 32]),
            seqno: Seqno::zero(),
            inbox_mmr,
        }
    }
}

impl ISnarkAccountState for TestSnarkState {
    fn new_fresh(update_vk: PredicateKey, initial_state_root: Hash) -> Self {
        let generic_mmr = CompactMmr64::<[u8; 32]>::new(64);
        let inbox_mmr = Mmr64::from_generic(&generic_mmr);
        Self {
            update_vk,
            inner_state_root: initial_state_root,
            seqno: Seqno::zero(),
            inbox_mmr,
        }
    }

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
    fn set_proof_state(&mut self, state: Hash, _next_read_idx: u64, seqno: Seqno) {
        self.inner_state_root = state;
        self.seqno = seqno;
    }

    fn insert_inbox_message(&mut self, _entry: MessageEntry) -> StateResult<()> {
        Ok(())
    }

    fn set_update_vk(&mut self, new_vk: PredicateKey) {
        self.update_vk = new_vk;
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
    type Write = Self;

    fn new_with_serial(new_acct_data: NewAccountData, serial: AccountSerial) -> Self {
        let balance = new_acct_data.initial_balance();
        let (ty, snark) = match new_acct_data.into_type_state() {
            NewAccountTypeState::Empty => (AccountTypeId::Empty, None),
            NewAccountTypeState::Snark {
                update_vk,
                initial_state_root,
            } => (
                AccountTypeId::Snark,
                Some(TestSnarkState::new_fresh(update_vk, initial_state_root)),
            ),
        };
        Self {
            serial,
            balance,
            ty,
            snark,
        }
    }

    fn apply_write(&mut self, write: Self::Write) -> StateResult<()> {
        *self = write;
        Ok(())
    }

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

    fn as_snark_account(&self) -> StateResult<&Self::SnarkAccountState> {
        self.snark.as_ref().ok_or(StateError::MismatchedAcctType {
            got: self.ty,
            expected: AccountTypeId::Snark,
        })
    }
}

impl IAccountStateMut for TestAccountState {
    type SnarkAccountStateMut = TestSnarkState;

    fn add_balance(&mut self, coin: Coin) {
        let new_balance = self.balance.to_sat() + coin.amt().to_sat();
        self.balance = BitcoinAmount::try_from(new_balance)
            .expect("amount must not exceed the Bitcoin money supply");
        coin.safely_consume_unchecked();
    }

    fn take_balance(&mut self, amt: BitcoinAmount) -> StateResult<Coin> {
        panic!("test: take_balance called in test for {amt}");
    }

    fn as_snark_account_mut(&mut self) -> StateResult<&mut Self::SnarkAccountStateMut> {
        self.snark.as_mut().ok_or(StateError::MismatchedAcctType {
            got: self.ty,
            expected: AccountTypeId::Snark,
        })
    }
}

#[derive(Debug)]
struct TestState {
    accounts: BTreeMap<AccountId, TestAccountState>,
    next_serial: AccountSerial,
    serial_overrides: VecDeque<AccountSerial>,
    cur_slot: u64,
    limbo_funds: BitcoinAmount,
    cur_epoch: u32,
    last_l1_blkid: L1BlockId,
    last_l1_height: L1Height,
    asm_recorded_epoch: EpochCommitment,
    total_ledger_balance: BitcoinAmount,
    pending_asm_logs: Vec<PendingAsmLog>,
}

impl TestState {
    fn new_with_serials(serials: Vec<AccountSerial>) -> Self {
        Self {
            accounts: BTreeMap::new(),
            next_serial: AccountSerial::one(),
            serial_overrides: VecDeque::from(serials),
            cur_slot: 0,
            limbo_funds: BitcoinAmount::default(),
            cur_epoch: 0,
            last_l1_blkid: L1BlockId::from(Buf32::zero()),
            last_l1_height: L1Height::from(0u32),
            asm_recorded_epoch: EpochCommitment::null(),
            total_ledger_balance: BitcoinAmount::default(),
            pending_asm_logs: Vec::new(),
        }
    }
}

impl IStateAccessor for TestState {
    type AccountState = TestAccountState;

    fn cur_slot(&self) -> u64 {
        self.cur_slot
    }

    fn limbo_funds(&self) -> BitcoinAmount {
        self.limbo_funds
    }

    fn cur_epoch(&self) -> u32 {
        self.cur_epoch
    }

    fn last_l1_blkid(&self) -> &L1BlockId {
        &self.last_l1_blkid
    }

    fn last_l1_height(&self) -> L1Height {
        self.last_l1_height
    }

    fn asm_recorded_epoch(&self) -> &EpochCommitment {
        &self.asm_recorded_epoch
    }

    fn total_ledger_balance(&self) -> BitcoinAmount {
        self.total_ledger_balance
    }

    fn check_account_exists(&self, id: AccountId) -> StateResult<bool> {
        Ok(self.accounts.contains_key(&id))
    }

    fn get_account_state(&self, id: AccountId) -> StateResult<Option<&Self::AccountState>> {
        Ok(self.accounts.get(&id))
    }

    fn find_account_id_by_serial(&self, serial: AccountSerial) -> StateResult<Option<AccountId>> {
        Ok(self
            .accounts
            .iter()
            .find_map(|(id, acct)| (acct.serial == serial).then_some(*id)))
    }

    fn next_account_serial(&self) -> AccountSerial {
        self.next_serial
    }

    fn compute_state_root(&self) -> StateResult<Buf32> {
        Ok(Buf32::zero())
    }

    fn l1_block_refs_mmr(&self) -> &Mmr64 {
        todo!()
    }

    fn pending_asm_logs_len(&self) -> usize {
        self.pending_asm_logs.len()
    }

    fn get_pending_asm_log(&self, idx: usize) -> Option<PendingAsmLog> {
        self.pending_asm_logs.get(idx).cloned()
    }

    fn pending_asm_logs_full(&self) -> bool {
        self.pending_asm_logs.len() as u64 == MAX_PENDING_ASM_LOGS
    }
}

impl IStateAccessorMut for TestState {
    type AccountStateMut = TestAccountState;

    fn set_cur_slot(&mut self, slot: u64) {
        self.cur_slot = slot;
    }

    fn add_limbo_funds_coin(&mut self, coin: Coin) -> StateResult<()> {
        let amt = coin.amt();
        let new = self
            .limbo_funds
            .to_sat()
            .checked_add(amt.to_sat())
            .and_then(|sats| BitcoinAmount::try_from(sats).ok());
        let Some(new) = new else {
            // Defuse the coin before returning so the mock upholds the same
            // consume-always contract as the real layers (dropping it panics).
            coin.safely_consume_unchecked();
            return Err(StateError::LimboFundsOverflow {
                cur: self.limbo_funds,
                add: amt,
            });
        };
        self.limbo_funds = new;
        coin.safely_consume_unchecked();
        Ok(())
    }

    fn take_limbo_funds_coin(&mut self, amt: BitcoinAmount) -> StateResult<Coin> {
        let new_sats = self.limbo_funds.to_sat().checked_sub(amt.to_sat()).ok_or(
            StateError::InsufficientLimboFunds {
                need: amt,
                have: self.limbo_funds,
            },
        )?;
        self.limbo_funds = BitcoinAmount::try_from(new_sats)
            .expect("subtracting from valid limbo funds must remain valid");
        Ok(Coin::new_unchecked(amt))
    }

    fn set_cur_epoch(&mut self, epoch: u32) {
        self.cur_epoch = epoch;
    }

    fn append_l1_block_rec(&mut self, _height: L1Height, _rec: L1BlockRecord) {}

    fn set_asm_recorded_epoch(&mut self, epoch: EpochCommitment) {
        self.asm_recorded_epoch = epoch;
    }

    fn set_total_ledger_balance(&mut self, amt: BitcoinAmount) {
        self.total_ledger_balance = amt;
    }

    fn update_account<R, F>(&mut self, id: AccountId, f: F) -> StateResult<R>
    where
        F: FnOnce(&mut Self::AccountStateMut) -> R,
    {
        let acct = self
            .accounts
            .get_mut(&id)
            .ok_or(StateError::MissingAccount(id))?;
        Ok(f(acct))
    }

    fn create_new_account(
        &mut self,
        id: AccountId,
        new_acct_data: NewAccountData,
    ) -> StateResult<AccountSerial> {
        if self.accounts.contains_key(&id) {
            return Err(StateError::AccountExists(id));
        }

        let serial = if let Some(serial) = self.serial_overrides.pop_front() {
            serial
        } else {
            let serial = self.next_serial;
            self.next_serial = self.next_serial.incr();
            serial
        };

        let acct = TestAccountState::new_with_serial(new_acct_data, serial);
        self.accounts.insert(id, acct);
        Ok(serial)
    }

    fn try_append_pending_asm_log(&mut self, entry: PendingAsmLog) -> StateResult<()> {
        if self.pending_asm_logs.len() as u64 == MAX_PENDING_ASM_LOGS {
            return Err(StateError::PendingAsmLogsFull);
        }
        self.pending_asm_logs.push(entry);
        Ok(())
    }

    fn reset_intraepoch_state(&mut self) {
        self.pending_asm_logs.clear();
    }
}

#[test]
fn test_da_blob_deterministic() {
    let blob1 = build_simple_blob();
    let blob2 = build_simple_blob();
    assert_eq!(blob1, blob2);
}

#[test]
fn test_account_diffs_ordered_by_serial() {
    let mut layer = create_test_base_layer();
    let account_id_1 = test_account_id(1);
    let account_id_2 = test_account_id(2);

    layer
        .create_new_account(
            account_id_1,
            test_new_snark_account_data(
                &test_snark_account_state(1),
                BitcoinAmount::try_from(1000)
                    .expect("amount must not exceed the Bitcoin money supply"),
            ),
        )
        .unwrap();
    layer
        .create_new_account(
            account_id_2,
            test_new_snark_account_data(
                &test_snark_account_state(2),
                BitcoinAmount::try_from(2000)
                    .expect("amount must not exceed the Bitcoin money supply"),
            ),
        )
        .unwrap();

    let mut da_state = DaAccumulatingState::new(layer);

    // Update higher serial first, then lower serial.
    da_state
        .update_account(account_id_2, |acct| {
            let coin = Coin::new_unchecked(
                BitcoinAmount::try_from(50)
                    .expect("amount must not exceed the Bitcoin money supply"),
            );
            acct.add_balance(coin);
        })
        .unwrap();
    da_state
        .update_account(account_id_1, |acct| {
            let coin = Coin::new_unchecked(
                BitcoinAmount::try_from(75)
                    .expect("amount must not exceed the Bitcoin money supply"),
            );
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
        BitcoinAmount::try_from(100).expect("amount must not exceed the Bitcoin money supply"),
        NewAccountTypeState::Snark {
            update_vk: snark_state.update_vk.clone(),
            initial_state_root: snark_state.inner_state_root,
        },
    );
    da_state.create_new_account(account_id, new_acct).unwrap();

    da_state
        .update_account(account_id, |acct| {
            let coin = Coin::new_unchecked(
                BitcoinAmount::try_from(50)
                    .expect("amount must not exceed the Bitcoin money supply"),
            );
            acct.add_balance(coin);
            acct.as_snark_account_mut()
                .unwrap()
                .set_proof_state(test_hash(9), 0, Seqno::new(1));
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
    assert_eq!(
        entry.init.balance,
        BitcoinAmount::try_from(150).expect("amount must not exceed the Bitcoin money supply")
    );
    match &entry.init.type_state {
        AccountTypeInitV1::Snark(init) => {
            assert_eq!(init.initial_state_root, test_hash(9));
            // The VK is stored with the predicate type ID prefix, so we need to compare
            // with the full predicate key bytes (type ID + raw VK bytes)
            let expected_vk =
                PredicateKey::try_new(PredicateTypeId::AlwaysAccept, update_vk.clone())
                    .expect("predicate condition must fit within the maximum length");
            assert_eq!(
                init.update_vk.as_slice(),
                expected_vk
                    .try_as_buf_ref()
                    .expect("predicate key must be valid")
                    .to_bytes()
            );
        }
        _ => panic!("expected snark account init"),
    }
    let diffs = blob.state_diff.ledger.account_diffs.entries();
    assert!(diffs.is_empty());
}

#[test]
fn test_new_account_vk_persisted_from_ol_state() {
    let mut da_state = DaAccumulatingState::new(create_test_base_layer());
    let account_id = test_account_id(10);
    let snark_state = OLSnarkAccountStateV1::new_fresh(PredicateKey::always_accept(), test_hash(4));
    let new_acct = NewAccountData::new(
        BitcoinAmount::try_from(100).expect("amount must not exceed the Bitcoin money supply"),
        NewAccountTypeState::Snark {
            update_vk: snark_state.update_vk().clone(),
            initial_state_root: snark_state.inner_state_root(),
        },
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
        AccountTypeInitV1::Snark(init) => {
            assert_eq!(
                init.update_vk.as_slice(),
                snark_state
                    .update_vk()
                    .try_as_buf_ref()
                    .expect("predicate key must be valid")
                    .to_bytes()
            );
        }
        _ => panic!("expected snark account init"),
    }
}

#[test]
fn test_take_resets_accumulator() {
    let account_id = test_account_id(1);
    let (layer, _) = setup_layer_with_snark_account(
        account_id,
        1,
        BitcoinAmount::try_from(1000).expect("amount must not exceed the Bitcoin money supply"),
    );
    let mut da_state = DaAccumulatingState::new(layer);

    // Finalize once after making changes.
    da_state
        .update_account(account_id, |acct| {
            let coin = Coin::new_unchecked(
                BitcoinAmount::try_from(123)
                    .expect("amount must not exceed the Bitcoin money supply"),
            );
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
fn test_limbo_funds_encoded_in_blob() {
    let mut da_state = DaAccumulatingState::new(create_test_base_layer());

    // Drive the limbo funds up then partially back down; the net change is what
    // the accumulator's limbo `DaCounter` should encode.
    da_state
        .add_limbo_funds_coin(Coin::new_unchecked(
            BitcoinAmount::try_from(1_000)
                .expect("amount must not exceed the Bitcoin money supply"),
        ))
        .unwrap();
    let taken = da_state
        .take_limbo_funds_coin(
            BitcoinAmount::try_from(400).expect("amount must not exceed the Bitcoin money supply"),
        )
        .unwrap();
    taken.safely_consume_unchecked();
    assert_eq!(
        da_state.limbo_funds(),
        BitcoinAmount::try_from(600).expect("amount must not exceed the Bitcoin money supply")
    );

    let blob_bytes = da_state
        .take_completed_epoch_da_blob()
        .expect("build DA blob")
        .expect("expected DA blob");
    let blob: OLDaPayloadV1 = decode_buf_exact(&blob_bytes).expect("decode DA blob");

    // The limbo counter records a net +600 change (not `new_unchanged`).
    let limbo_diff = blob
        .state_diff
        .global
        .limbo_funds_sats
        .diff()
        .expect("limbo funds changed");
    assert!(limbo_diff.is_positive());
    assert_eq!(limbo_diff.magnitude(), 600);
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
            BitcoinAmount::try_from(0).expect("amount must not exceed the Bitcoin money supply"),
            NewAccountTypeState::Snark {
                update_vk: snark_state.update_vk.clone(),
                initial_state_root: snark_state.inner_state_root,
            },
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
fn test_vk_size_at_predicate_limit_roundtrips() {
    let mut da_state = DaAccumulatingState::new(TestState::new_with_serials(vec![]));
    let account_id = test_account_id(1);
    let vk_len = MAX_CONDITION_LEN as usize;
    let snark_state = TestSnarkState::new(vec![0u8; vk_len]);
    let new_acct = NewAccountData::new(
        BitcoinAmount::try_from(0).expect("amount must not exceed the Bitcoin money supply"),
        NewAccountTypeState::Snark {
            update_vk: snark_state.update_vk.clone(),
            initial_state_root: snark_state.inner_state_root,
        },
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
        AccountTypeInitV1::Snark(init) => {
            assert_eq!(init.update_vk.as_slice().len(), vk_len + 1);
        }
        _ => panic!("expected snark account init"),
    }
}

#[test]
fn test_estimated_encoded_size_scales_with_new_account_vk_len() {
    let estimate_for = |vk_len: usize| {
        let mut da_state = DaAccumulatingState::new(TestState::new_with_serials(vec![]));
        let account_id = test_account_id(1);
        let snark_state = TestSnarkState::new(vec![0u8; vk_len]);
        let new_acct = NewAccountData::new(
            BitcoinAmount::try_from(0).expect("amount must not exceed the Bitcoin money supply"),
            NewAccountTypeState::Snark {
                update_vk: snark_state.update_vk.clone(),
                initial_state_root: snark_state.inner_state_root,
            },
        );
        da_state.create_new_account(account_id, new_acct).unwrap();
        da_state.accumulator().estimated_encoded_size()
    };

    let small_vk_len = 8;
    let large_vk_len = 512;
    let grown = estimate_for(large_vk_len) - estimate_for(small_vk_len);
    assert_eq!(
        grown,
        large_vk_len - small_vk_len,
        "estimate must scale with the new account's VK length"
    );
}

#[test]
fn test_oversized_predicate_key_is_rejected() {
    let oversized_vk_len = MAX_CONDITION_LEN as usize + 1;
    assert!(
        PredicateKey::try_new(PredicateTypeId::AlwaysAccept, vec![0u8; oversized_vk_len]).is_err()
    );
}

#[test]
fn test_message_source_missing_is_rejected() {
    let account_id = test_account_id(1);
    let (layer, _) = setup_layer_with_snark_account(
        account_id,
        1,
        BitcoinAmount::try_from(1_000).expect("amount must not exceed the Bitcoin money supply"),
    );
    let mut da_state = DaAccumulatingState::new(layer);

    let payload = MsgPayload::from_bytes(
        BitcoinAmount::try_from(0).expect("amount must not exceed the Bitcoin money supply"),
        vec![0u8; 4],
    )
    .expect("message payload bytes must fit within SSZ max length");
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
fn test_special_message_source_is_encoded() {
    let account_id = test_account_id(1);
    let (layer, _) = setup_layer_with_snark_account(
        account_id,
        1,
        BitcoinAmount::try_from(1_000).expect("amount must not exceed the Bitcoin money supply"),
    );
    let mut da_state = DaAccumulatingState::new(layer);

    let payload = MsgPayload::from_bytes(
        BitcoinAmount::try_from(0).expect("amount must not exceed the Bitcoin money supply"),
        vec![0u8; 4],
    )
    .expect("message payload bytes must fit within SSZ max length");
    let special_source = AccountId::special(0x10);
    let msg = MessageEntry::new(special_source, 0, payload);
    da_state
        .update_account(account_id, |acct| {
            acct.as_snark_account_mut()
                .unwrap()
                .insert_inbox_message(msg)
        })
        .unwrap()
        .unwrap();

    let blob_bytes = da_state
        .take_completed_epoch_da_blob()
        .expect("build DA blob")
        .expect("expected DA blob");
    let blob: OLDaPayloadV1 = decode_buf_exact(&blob_bytes).expect("decode DA blob");
    let diffs = blob.state_diff.ledger.account_diffs.entries();
    assert_eq!(diffs.len(), 1);
    let entries = diffs[0].diff.snark.inbox.new_entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].source, special_source);
}

#[test]
fn test_message_payload_size_limit() {
    let account_id = test_account_id(1);
    let (layer, _) = setup_layer_with_snark_account(
        account_id,
        1,
        BitcoinAmount::try_from(1_000).expect("amount must not exceed the Bitcoin money supply"),
    );
    let mut da_state = DaAccumulatingState::new(layer);

    let payload = MsgPayload::from_bytes(
        BitcoinAmount::try_from(0).expect("amount must not exceed the Bitcoin money supply"),
        vec![0u8; MAX_MSG_PAYLOAD_BYTES + 1],
    )
    .expect("message payload bytes must fit within SSZ max length");
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
    let new_acct = NewAccountData::new_empty(NewAccountTypeState::Empty);
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
    let new_acct = NewAccountData::new_empty(NewAccountTypeState::Empty);
    da_state.create_new_account(account_id, new_acct).unwrap();

    let result = da_state.take_completed_epoch_da_blob();
    assert!(matches!(
        result,
        Err(DaAccumulationError::NewAccountSerialGap(_, _))
    ));
}

// =============================================================================
// Tests verifying layer isolation
// =============================================================================

/// Test that modifications through combined layers don't affect the base state.
#[test]
fn test_combined_layers_preserve_base_state() {
    let account_id = test_account_id(1);
    let initial_balance =
        BitcoinAmount::try_from(1000).expect("amount must not exceed the Bitcoin money supply");
    let (base_layer, _) = setup_layer_with_snark_account(account_id, 1, initial_balance);

    // Save original values
    let original_slot = base_layer.cur_slot();
    let original_epoch = base_layer.cur_epoch();
    let original_inbox_count = base_layer
        .get_account_state(account_id)
        .unwrap()
        .unwrap()
        .as_snark_account()
        .unwrap()
        .inbox_mmr()
        .num_entries();

    let tracking = WriteTrackingState::new_empty(&base_layer);
    let mut indexer = IndexerState::new(tracking);

    // Make various modifications
    indexer.set_cur_slot(999);
    indexer.set_cur_epoch(99);
    indexer
        .update_account(account_id, |acct| {
            let coin = Coin::new_unchecked(
                BitcoinAmount::try_from(500)
                    .expect("amount must not exceed the Bitcoin money supply"),
            );
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
    assert_eq!(base_layer.cur_slot(), original_slot);
    assert_eq!(base_layer.cur_epoch(), original_epoch);

    let account = base_layer.get_account_state(account_id).unwrap().unwrap();
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
