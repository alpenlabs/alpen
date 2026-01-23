//! OL state accessor that accumulates DA-covered writes over an epoch.

use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    mem::take,
};

use strata_acct_types::{AccountId, AccountTypeId, AcctResult, BitcoinAmount, Mmr64};
use strata_checkpoint_types_ssz::OL_DA_DIFF_MAX_SIZE;
use strata_da_framework::{
    DaBuilder, DaCounterBuilder, DaQueueBuilder, DaRegister, DaWrite, counter_schemes::CtrU64ByU16,
    encode_to_vec,
};
use strata_identifiers::{AccountSerial, EpochCommitment, L1BlockId, L1Height};
use strata_ledger_types::{
    AccountTypeState, AccountTypeStateRef, IAccountState, IAccountStateMut, ISnarkAccountState,
    IStateAccessor, NewAccountData,
};
use strata_ol_chain_types_new::SimpleWithdrawalIntentLogData;
use strata_ol_da::{
    AccountDiff, AccountDiffEntry, AccountInit, AccountTypeInit, DaMessageEntry, DaProofState,
    InboxAccumulator, LedgerDiff, MAX_MSG_PAYLOAD_BYTES, MAX_VK_BYTES, NewAccountEntry,
    OLDaPayloadV1, PendingWithdrawQueue, SnarkAccountDiff, SnarkAccountInit, StateDiff, U16LenList,
};
use strata_ol_msg_types::MAX_WITHDRAWAL_DESC_LEN;
use thiserror::Error;

use crate::{
    index_types::{IndexerWrites, SnarkAcctStateUpdate},
    indexer_layer::IndexerAccountStateMut,
};

/// Errors while building or encoding epoch DA payloads.
#[derive(Debug, Error)]
pub enum DaAccumulationError {
    /// Error while building DA writes for the epoch.
    #[error("da accumulator builder error: {0}")]
    Builder(#[from] strata_da_framework::BuilderError),

    /// Account state missing when assembling diffs.
    #[error("da accumulator missing account {0}")]
    MissingAccount(AccountId),

    /// Missing pre-state snapshot for a touched account.
    #[error("da accumulator missing pre-state {0}")]
    MissingPreState(AccountId),

    /// Duplicate account serial encountered when ordering diffs.
    #[error("da accumulator duplicate account serial {0}")]
    DuplicateAccountSerial(AccountSerial),

    /// New account serials are not contiguous.
    #[error("da accumulator serial gap expected {0} got {1}")]
    NewAccountSerialGap(AccountSerial, AccountSerial),

    /// VK size exceeds maximum allowed.
    #[error("da accumulator vk too large: {provided} bytes (max {max})")]
    VkTooLarge { provided: usize, max: usize },

    /// Withdrawal intent destination exceeds maximum allowed.
    #[error("da accumulator withdrawal dest too large: {provided} bytes (max {max})")]
    WithdrawalIntentTooLarge { provided: usize, max: usize },

    /// Message payload exceeds maximum allowed.
    #[error("da accumulator message payload too large: {provided} bytes (max {max})")]
    MessagePayloadTooLarge { provided: usize, max: usize },

    /// Encoded DA blob exceeds the maximum size limit.
    #[error("da accumulator payload too large: {provided} bytes (max {max})")]
    PayloadTooLarge { provided: usize, max: u64 },
}

// ============================================================================
// Accumulator data
// ============================================================================

/// Snapshot of snark account fields needed for diffing.
#[derive(Clone, Debug)]
struct SnarkSnapshot {
    /// Sequence number at the start of DA-covered execution.
    seq_no: u64,
}

/// Snapshot of an account before DA-covered execution.
#[derive(Clone, Debug)]
struct AccountSnapshot {
    /// Account balance at the start of DA-covered execution.
    balance: BitcoinAmount,

    /// Account type at the start of DA-covered execution.
    ty: AccountTypeId,

    /// Snark snapshot if the account is a snark account.
    snark: Option<SnarkSnapshot>,
}

impl AccountSnapshot {
    fn from_state<T: IAccountState>(state: &T) -> AcctResult<Self> {
        let ty = state.ty();
        let snark = match state.type_state() {
            AccountTypeStateRef::Snark(snark_state) => Some(SnarkSnapshot {
                seq_no: *snark_state.seqno().inner(),
            }),
            AccountTypeStateRef::Empty => None,
        };
        Ok(Self {
            balance: state.balance(),
            ty,
            snark,
        })
    }
}

/// Per-epoch accumulator of DA writes before encoding.
#[derive(Default, Debug)]
struct EpochDaAccumulator {
    /// Slot value at the start of the epoch.
    slot_base: Option<u64>,

    /// Final slot value seen during the epoch.
    slot_final: Option<u64>,

    /// First serial assigned in this epoch, used to enforce contiguity.
    first_new_serial: Option<AccountSerial>,

    /// New account entries created during the epoch.
    new_accounts: Vec<NewAccountEntry>,

    /// Account IDs created during the epoch.
    new_account_ids: BTreeSet<AccountId>,

    /// Accounts touched during the epoch (for diff generation).
    touched_accounts: BTreeSet<AccountId>,

    /// Pre-execution snapshots for touched accounts.
    pre_states: BTreeMap<AccountId, AccountSnapshot>,

    /// Inbox messages appended during the epoch.
    inbox_messages: BTreeMap<AccountId, Vec<DaMessageEntry>>,

    /// Snark state updates recorded during the epoch.
    snark_updates: BTreeMap<AccountId, Vec<SnarkAcctStateUpdate>>,

    /// Withdrawal intents collected during the epoch.
    withdrawal_intents: Vec<SimpleWithdrawalIntentLogData>,

    /// Pending withdrawals queue snapshot at the start of the epoch.
    pending_withdraw_source: PendingWithdrawQueue,

    /// Pending withdrawals queue front increments recorded during the epoch.
    pending_withdraw_front_incr: u16,

    /// Last error encountered while building a blob.
    last_error: Option<DaAccumulationError>,
}

impl EpochDaAccumulator {
    /// Records a slot change event.
    fn record_slot_change(&mut self, prior: u64, new: u64) {
        if self.slot_base.is_none() {
            self.slot_base = Some(prior);
        }
        self.slot_final = Some(new);
    }

    /// Records the pre-state of an account.
    fn record_pre_state<T: IAccountState>(
        &mut self,
        account_id: AccountId,
        state: &T,
    ) -> AcctResult<()> {
        if !self.pre_states.contains_key(&account_id) {
            let snapshot = AccountSnapshot::from_state(state)?;
            if let Entry::Vacant(entry) = self.pre_states.entry(account_id) {
                entry.insert(snapshot);
            }
        }
        Ok(())
    }

    /// Records the inbox messages and snark state updates from an indexer write.
    fn record_writes(&mut self, writes: IndexerWrites) {
        for msg in writes.inbox_messages() {
            let payload_len = msg.entry.payload().data().len();
            if payload_len > MAX_MSG_PAYLOAD_BYTES {
                self.last_error = Some(DaAccumulationError::MessagePayloadTooLarge {
                    provided: payload_len,
                    max: MAX_MSG_PAYLOAD_BYTES,
                });
                continue;
            }
            let entry = DaMessageEntry::from(msg.entry.clone());
            self.inbox_messages
                .entry(msg.account_id)
                .or_default()
                .push(entry);
        }

        for update in writes.snark_state_updates() {
            self.snark_updates
                .entry(update.account_id())
                .or_default()
                .push(update.clone());
        }
    }

    /// Records a withdrawal intent.
    fn record_withdrawal_intent(&mut self, intent: SimpleWithdrawalIntentLogData) {
        self.withdrawal_intents.push(intent);
    }

    /// Records a pending withdrawal queue snapshot.
    fn record_pending_withdraw_queue(&mut self, pending: PendingWithdrawQueue) {
        self.pending_withdraw_source = pending;
    }

    /// Records a pending withdrawals front increment.
    fn record_pending_withdraw_front_incr(&mut self, incr: u16) {
        if incr == 0 {
            return;
        }
        let Some(new_total) = self.pending_withdraw_front_incr.checked_add(incr) else {
            if self.last_error.is_none() {
                self.last_error = Some(DaAccumulationError::Builder(
                    strata_da_framework::BuilderError::OutOfBoundsValue,
                ));
            }
            return;
        };
        self.pending_withdraw_front_incr = new_total;
    }

    /// Records a new account.
    fn record_new_account(&mut self, serial: AccountSerial, entry: NewAccountEntry) {
        if let Some(first_serial) = self.first_new_serial {
            let expected =
                AccountSerial::new(*first_serial.inner() + self.new_accounts.len() as u32);
            if serial != expected && self.last_error.is_none() {
                self.last_error = Some(DaAccumulationError::NewAccountSerialGap(expected, serial));
            }
        } else {
            self.first_new_serial = Some(serial);
        }
        if let AccountTypeInit::Snark(init) = &entry.init.type_state {
            let vk_len = init.update_vk.as_slice().len();
            if vk_len > MAX_VK_BYTES {
                self.last_error = Some(DaAccumulationError::VkTooLarge {
                    provided: vk_len,
                    max: MAX_VK_BYTES,
                });
            }
        }
        self.new_account_ids.insert(entry.account_id);
        self.new_accounts.push(entry);
    }

    /// Records a touched account.
    fn record_touched_account(&mut self, account_id: AccountId) {
        self.touched_accounts.insert(account_id);
    }

    /// Finalizes the epoch by building the DA blob.
    fn finalize<S: IStateAccessor>(&mut self, state: &S) -> Result<Vec<u8>, DaAccumulationError> {
        if let Some(err) = self.last_error.take() {
            return Err(err);
        }
        let global_diff = self.build_global_diff()?;
        let ledger_diff = self.build_ledger_diff(state)?;
        let state_diff = StateDiff::new(global_diff, ledger_diff);
        let blob = OLDaPayloadV1::new(state_diff);

        let encoded = encode_to_vec(&blob).map_err(|_| {
            // encode_to_vec only returns CodecError; map to builder error for now
            DaAccumulationError::Builder(strata_da_framework::BuilderError::OutOfBoundsValue)
        })?;

        if encoded.len() as u64 > OL_DA_DIFF_MAX_SIZE {
            return Err(DaAccumulationError::PayloadTooLarge {
                provided: encoded.len(),
                max: OL_DA_DIFF_MAX_SIZE,
            });
        }

        // Return the encoded DA blob.
        Ok(encoded)
    }

    /// Builds the global state diff for the epoch.
    fn build_global_diff(&self) -> Result<strata_ol_da::GlobalStateDiff, DaAccumulationError> {
        let cur_slot = if let (Some(base), Some(final_slot)) = (self.slot_base, self.slot_final) {
            let mut builder = DaCounterBuilder::<CtrU64ByU16>::from_source(base);
            builder.set(final_slot)?;
            builder.into_write()?
        } else {
            strata_da_framework::DaCounter::new_unchanged()
        };

        let mut queue_builder = DaQueueBuilder::<PendingWithdrawQueue>::from_source(
            self.pending_withdraw_source.clone(),
        );
        for intent in &self.withdrawal_intents {
            if !queue_builder.append_entry(intent.clone()) {
                return Err(DaAccumulationError::Builder(
                    strata_da_framework::BuilderError::OutOfBoundsValue,
                ));
            }
        }
        if !queue_builder.add_front_incr(self.pending_withdraw_front_incr) {
            return Err(DaAccumulationError::Builder(
                strata_da_framework::BuilderError::OutOfBoundsValue,
            ));
        }
        let pending_withdraws = queue_builder.into_write()?;

        Ok(strata_ol_da::GlobalStateDiff::new(
            cur_slot,
            pending_withdraws,
        ))
    }

    /// Builds the ledger diff for the epoch.
    fn build_ledger_diff<S: IStateAccessor>(
        &self,
        state: &S,
    ) -> Result<LedgerDiff, DaAccumulationError> {
        let mut new_records = self.new_accounts.clone();
        new_records.sort_by_key(|entry| entry.serial);

        if let Some(mut expected) = self.first_new_serial {
            for entry in &new_records {
                if entry.serial != expected {
                    return Err(DaAccumulationError::NewAccountSerialGap(
                        expected,
                        entry.serial,
                    ));
                }
                expected = expected.incr();
            }
        }

        let mut new_accounts = Vec::with_capacity(new_records.len());
        for entry in &new_records {
            let post = state
                .get_account_state(entry.account_id)
                .map_err(|_| DaAccumulationError::MissingAccount(entry.account_id))?
                .ok_or(DaAccumulationError::MissingAccount(entry.account_id))?;
            let init = account_init_from_state(post)?;
            new_accounts.push(NewAccountEntry::new(entry.serial, entry.account_id, init));
        }

        let mut account_diffs = Vec::new();
        let mut seen_serials = BTreeSet::new();

        for account_id in &self.touched_accounts {
            if self.new_account_ids.contains(account_id) {
                continue;
            }

            let pre = self
                .pre_states
                .get(account_id)
                .ok_or(DaAccumulationError::MissingPreState(*account_id))?;
            let post = state
                .get_account_state(*account_id)
                .map_err(|_| DaAccumulationError::MissingAccount(*account_id))?
                .ok_or(DaAccumulationError::MissingAccount(*account_id))?;

            let balance = DaRegister::compare(&pre.balance, &post.balance());
            let snark_state = self.build_snark_diff(pre, post, *account_id)?;
            let diff = AccountDiff::new(balance, snark_state);

            if <AccountDiff as DaWrite>::is_default(&diff) {
                continue;
            }

            let serial = post.serial();
            if !seen_serials.insert(serial) {
                return Err(DaAccumulationError::DuplicateAccountSerial(serial));
            }

            account_diffs.push(AccountDiffEntry::new(serial, diff));
        }

        account_diffs.sort_by_key(|entry| entry.account_serial);

        Ok(LedgerDiff::new(
            U16LenList::new(new_accounts),
            U16LenList::new(account_diffs),
        ))
    }

    /// Builds the snark account diff for the epoch.
    fn build_snark_diff<T: IAccountState>(
        &self,
        pre: &AccountSnapshot,
        post: &T,
        account_id: AccountId,
    ) -> Result<SnarkAccountDiff, DaAccumulationError> {
        if pre.ty != strata_identifiers::AccountTypeId::Snark {
            return Ok(SnarkAccountDiff::default());
        }

        let post_snark = post
            .as_snark_account()
            .map_err(|_| DaAccumulationError::MissingAccount(account_id))?;
        let post_seq = *post_snark.seqno().inner();

        let pre_seq = pre.snark.as_ref().map(|s| s.seq_no).unwrap_or(0);
        let mut seq_builder = DaCounterBuilder::<CtrU64ByU16>::from_source(pre_seq);
        seq_builder.set(post_seq)?;
        let seq_no = seq_builder.into_write()?;

        let proof_state = if let Some(updates) = self.snark_updates.get(&account_id) {
            if let Some(last) = updates.last() {
                let state = last.state();
                let next_read = last.next_read_idx();
                DaRegister::new_set(DaProofState::new(state, next_read))
            } else {
                DaRegister::new_unset()
            }
        } else {
            DaRegister::new_unset()
        };

        let mut inbox = strata_da_framework::DaLinacc::<InboxAccumulator>::new();
        if let Some(msgs) = self.inbox_messages.get(&account_id) {
            for msg in msgs {
                if !inbox.append_entry(msg.clone()) {
                    return Err(DaAccumulationError::Builder(
                        strata_da_framework::BuilderError::OutOfBoundsValue,
                    ));
                }
            }
        }

        Ok(SnarkAccountDiff::new(seq_no, proof_state, inbox))
    }
}

/// State accessor that accumulates DA-covered writes for a single epoch.
#[derive(Debug)]
pub struct DaAccumulatingState<S: IStateAccessor> {
    /// Wrapped state accessor.
    inner: S,

    /// Toggle for recording DA-covered writes.
    da_tracking_enabled: bool,

    /// Epoch-scoped DA write accumulator.
    epoch_acc: EpochDaAccumulator,
}

impl<S: IStateAccessor> DaAccumulatingState<S> {
    /// Creates a new DA accumulating state accessor.
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            da_tracking_enabled: true,
            epoch_acc: EpochDaAccumulator::default(),
        }
    }

    /// Returns a reference to the wrapped state accessor.
    pub fn inner(&self) -> &S {
        &self.inner
    }

    /// Returns a mutable reference to the wrapped state accessor.
    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.inner
    }

    /// Returns the next completed epoch DA blob, if any.
    pub fn take_completed_epoch_da_blob(&mut self) -> Option<Vec<u8>> {
        if !self.da_tracking_enabled {
            return None;
        }

        let mut acc = take(&mut self.epoch_acc);
        match acc.finalize(&self.inner) {
            Ok(blob) => Some(blob),
            Err(err) => {
                self.epoch_acc.last_error = Some(err);
                None
            }
        }
    }

    /// Returns the last DA accumulation error, if any.
    pub fn last_error(&self) -> Option<&DaAccumulationError> {
        self.epoch_acc.last_error.as_ref()
    }

    /// Records the pending withdrawal queue snapshot for the epoch.
    pub fn record_pending_withdraw_queue(&mut self, queue: PendingWithdrawQueue) {
        if self.da_tracking_enabled {
            self.epoch_acc.record_pending_withdraw_queue(queue);
        }
    }

    /// Records a front increment for the pending withdrawal queue.
    pub fn record_pending_withdraw_front_incr(&mut self, incr: u16) {
        if self.da_tracking_enabled {
            self.epoch_acc.record_pending_withdraw_front_incr(incr);
        }
    }
}

impl<S> IStateAccessor for DaAccumulatingState<S>
where
    S: IStateAccessor,
    S::AccountState: IAccountState,
    S::AccountStateMut: Clone,
    <S::AccountStateMut as IAccountStateMut>::SnarkAccountStateMut: Clone,
{
    type AccountState = S::AccountState;
    type AccountStateMut = IndexerAccountStateMut<S::AccountStateMut>;

    // ===== Global state methods =====

    fn cur_slot(&self) -> u64 {
        self.inner.cur_slot()
    }

    fn set_cur_slot(&mut self, slot: u64) {
        if self.da_tracking_enabled {
            let prior = self.inner.cur_slot();
            self.epoch_acc.record_slot_change(prior, slot);
        }
        self.inner.set_cur_slot(slot);
    }

    // ===== Epochal state methods =====

    fn cur_epoch(&self) -> u32 {
        self.inner.cur_epoch()
    }

    fn set_cur_epoch(&mut self, epoch: u32) {
        let prev = self.inner.cur_epoch();
        if self.da_tracking_enabled && epoch != prev {
            panic!("da accumulating state cannot span epochs while tracking is enabled");
        }
        self.inner.set_cur_epoch(epoch);
    }

    fn last_l1_blkid(&self) -> &L1BlockId {
        self.inner.last_l1_blkid()
    }

    fn last_l1_height(&self) -> L1Height {
        self.inner.last_l1_height()
    }

    fn append_manifest(&mut self, height: L1Height, mf: strata_asm_manifest_types::AsmManifest) {
        self.inner.append_manifest(height, mf);
    }

    fn asm_recorded_epoch(&self) -> &EpochCommitment {
        self.inner.asm_recorded_epoch()
    }

    fn set_asm_recorded_epoch(&mut self, epoch: EpochCommitment) {
        self.inner.set_asm_recorded_epoch(epoch);
    }

    fn total_ledger_balance(&self) -> BitcoinAmount {
        self.inner.total_ledger_balance()
    }

    fn set_total_ledger_balance(&mut self, amt: BitcoinAmount) {
        self.inner.set_total_ledger_balance(amt);
    }

    // ===== Account methods =====

    fn check_account_exists(&self, id: AccountId) -> AcctResult<bool> {
        self.inner.check_account_exists(id)
    }

    fn get_account_state(&self, id: AccountId) -> AcctResult<Option<&Self::AccountState>> {
        self.inner.get_account_state(id)
    }

    fn update_account<R, F>(&mut self, id: AccountId, f: F) -> AcctResult<R>
    where
        F: FnOnce(&mut Self::AccountStateMut) -> R,
    {
        if self.da_tracking_enabled
            && let Some(account_state) = self.inner.get_account_state(id)?
        {
            self.epoch_acc.record_pre_state(id, account_state)?;
            self.epoch_acc.record_touched_account(id);
        }

        let (result, local_writes) = self.inner.update_account(id, |inner_acct| {
            let mut wrapped = IndexerAccountStateMut::new(inner_acct.clone(), id);
            let user_result = f(&mut wrapped);
            let (modified_inner, writes, was_modified) = wrapped.into_parts();
            if was_modified {
                *inner_acct = modified_inner;
            }
            (user_result, writes)
        })?;

        if self.da_tracking_enabled {
            self.epoch_acc.record_writes(local_writes);
        }

        Ok(result)
    }

    fn create_new_account(
        &mut self,
        id: AccountId,
        new_acct_data: NewAccountData<Self::AccountState>,
    ) -> AcctResult<AccountSerial> {
        let init = if self.da_tracking_enabled {
            Some(account_init_from_data(&new_acct_data))
        } else {
            None
        };

        let serial = self.inner.create_new_account(id, new_acct_data)?;

        if let Some(init) = init {
            let entry = NewAccountEntry::new(serial, id, init);
            self.epoch_acc.record_new_account(serial, entry);
        }

        Ok(serial)
    }

    fn find_account_id_by_serial(&self, serial: AccountSerial) -> AcctResult<Option<AccountId>> {
        self.inner.find_account_id_by_serial(serial)
    }

    fn next_account_serial(&self) -> AccountSerial {
        self.inner.next_account_serial()
    }

    fn compute_state_root(&self) -> AcctResult<strata_identifiers::Buf32> {
        self.inner.compute_state_root()
    }

    fn asm_manifests_mmr(&self) -> &Mmr64 {
        self.inner.asm_manifests_mmr()
    }

    fn record_withdrawal_intent(&mut self, amt: u64, dest: Vec<u8>) {
        self.inner.record_withdrawal_intent(amt, dest.clone());

        if !self.da_tracking_enabled {
            return;
        }

        let dest_len = dest.len();

        let Some(intent) = SimpleWithdrawalIntentLogData::new(amt, dest) else {
            #[cfg(feature = "tracing")]
            tracing::warn!("failed to record withdrawal intent (dest too large)");
            return;
        };

        if dest_len > MAX_WITHDRAWAL_DESC_LEN {
            self.epoch_acc.last_error = Some(DaAccumulationError::WithdrawalIntentTooLarge {
                provided: dest_len,
                max: MAX_WITHDRAWAL_DESC_LEN,
            });
        }

        self.epoch_acc.record_withdrawal_intent(intent);
    }
}

/// Converts new-account data into DA init data for encoding.
fn account_init_from_data<T: IAccountState>(data: &NewAccountData<T>) -> AccountInit {
    let balance = data.initial_balance();
    match data.type_state() {
        AccountTypeState::Empty => AccountInit::new(balance, AccountTypeInit::Empty),
        AccountTypeState::Snark(snark_state) => {
            let init = SnarkAccountInit::new(
                snark_state.inner_state_root(),
                snark_state.update_vk().as_buf_ref().to_bytes(),
            );
            AccountInit::new(balance, AccountTypeInit::Snark(init))
        }
    }
}

/// Converts post-state into DA init data for encoding new accounts.
fn account_init_from_state<T: IAccountState>(
    state: &T,
) -> Result<AccountInit, DaAccumulationError> {
    let balance = state.balance();
    match state.type_state() {
        AccountTypeStateRef::Empty => Ok(AccountInit::new(balance, AccountTypeInit::Empty)),
        AccountTypeStateRef::Snark(snark_state) => {
            let vk = snark_state.update_vk().as_buf_ref().to_bytes();
            if vk.len() > MAX_VK_BYTES {
                return Err(DaAccumulationError::VkTooLarge {
                    provided: vk.len(),
                    max: MAX_VK_BYTES,
                });
            }
            let init = SnarkAccountInit::new(snark_state.inner_state_root(), vk.to_vec());
            Ok(AccountInit::new(balance, AccountTypeInit::Snark(init)))
        }
    }
}
