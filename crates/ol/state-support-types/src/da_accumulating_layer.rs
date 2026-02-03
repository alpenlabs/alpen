//! OL state accessor that accumulates DA-covered writes over an epoch.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt,
    mem::take,
};

use strata_acct_types::{AccountId, AccountTypeId, AcctResult, BitcoinAmount, Mmr64};
use strata_checkpoint_types_ssz::OL_DA_DIFF_MAX_SIZE;
use strata_da_framework::{
    CodecError, DaBuilder, DaCounter, DaCounterBuilder, DaLinacc, DaRegister, LinearAccumulator,
    counter_schemes::{CtrU64ByU16, CtrU64ByUnsignedVarint},
    encode_to_vec,
};
use strata_identifiers::{AccountSerial, EpochCommitment, L1BlockId, L1Height};
use strata_ledger_types::{
    AccountTypeStateRef, IAccountState, IAccountStateMut, ISnarkAccountState, IStateAccessor,
    NewAccountData,
};
use strata_ol_da::{
    AccountDiff, AccountDiffEntry, AccountInit, AccountTypeInit, DaMessageEntry, DaProofState,
    DaProofStateDiff, GlobalStateDiff, InboxBuffer, LedgerDiff, MAX_MSG_PAYLOAD_BYTES,
    MAX_VK_BYTES, NewAccountEntry, OLDaPayloadV1, SnarkAccountDiff, SnarkAccountInit, StateDiff,
    U16LenList,
};
use thiserror::Error;

use crate::{index_types::IndexerWrites, indexer_layer::IndexerAccountStateMut};

/// Errors while building or encoding epoch DA payloads.
#[derive(Debug, Error)]
pub enum DaAccumulationError {
    /// Error while building DA writes for the epoch.
    #[error("da accumulator builder error: {0}")]
    Builder(#[from] strata_da_framework::BuilderError),

    /// Error while encoding DA blob.
    #[error("da accumulator codec error: {0}")]
    Codec(#[from] CodecError),

    /// Account state missing when assembling diffs.
    #[error("da accumulator missing account {0}")]
    MissingAccountState(AccountId),

    /// Missing pre-state snapshot for a touched account.
    #[error("da accumulator missing pre-state {0}")]
    MissingPreState(AccountId),

    /// Account type changed during the epoch.
    #[error(
        "da accumulator account type changed for {account_id} (expected {expected}, got {actual})"
    )]
    AccountTypeChanged {
        account_id: AccountId,
        expected: AccountTypeId,
        actual: AccountTypeId,
    },

    /// Snark state missing for a snark diff.
    #[error("da accumulator missing snark state {0}")]
    MissingSnarkState(AccountId),

    /// Inbox message targeted unexpected account.
    #[error("da accumulator inbox account mismatch (expected {expected}, got {actual})")]
    InboxAccountMismatch {
        expected: AccountId,
        actual: AccountId,
    },

    /// Inbox message source is missing from ledger state.
    #[error("da accumulator missing message source {0}")]
    MessageSourceMissing(AccountId),

    /// Duplicate account serial encountered when ordering diffs.
    #[error("da accumulator duplicate account serial {0}")]
    DuplicateAccountSerial(AccountSerial),

    /// Duplicate new account ID encountered while building new account list.
    #[error("da accumulator duplicate new account id {0}")]
    DuplicateNewAccountId(AccountId),

    /// New account serials are not contiguous.
    #[error("da accumulator serial gap expected {0} got {1}")]
    NewAccountSerialGap(AccountSerial, AccountSerial),

    /// VK size exceeds maximum allowed.
    #[error("da accumulator vk too large: {provided} bytes (max {max})")]
    VkTooLarge { provided: usize, max: usize },

    /// Message payload exceeds maximum allowed.
    #[error("da accumulator message payload too large: {provided} bytes (max {max})")]
    MessagePayloadTooLarge { provided: usize, max: usize },

    /// Inbox buffer exceeded maximum message count.
    #[error("da accumulator inbox buffer full: account {account_id} exceeded {max} messages")]
    InboxBufferFull { account_id: AccountId, max: u16 },

    /// Encoded DA blob exceeds the maximum size limit.
    #[error("da accumulator payload too large: {provided} bytes (max {max})")]
    PayloadTooLarge { provided: usize, max: u64 },
}

// ============================================================================
// Accumulator data
// ============================================================================

/// Tracked snark account fields needed for diffing.
#[derive(Clone, Debug)]
struct SnarkDelta {
    base_seq_no: u64,
    final_seq_no: u64,
    base_proof_state: DaProofState,
    final_proof_state: DaProofState,
    inbox: DaLinacc<InboxBuffer>,
}

impl SnarkDelta {
    fn from_state<T: ISnarkAccountState>(state: &T) -> Self {
        let base_seq_no = *state.seqno().inner();
        let base_proof_state =
            DaProofState::new(state.inner_state_root(), state.next_inbox_msg_idx());
        Self {
            base_seq_no,
            final_seq_no: base_seq_no,
            base_proof_state: base_proof_state.clone(),
            final_proof_state: base_proof_state,
            inbox: DaLinacc::new(),
        }
    }

    fn update_from_state<T: ISnarkAccountState>(&mut self, state: &T) {
        self.final_seq_no = *state.seqno().inner();
        self.final_proof_state =
            DaProofState::new(state.inner_state_root(), state.next_inbox_msg_idx());
    }

    fn build_diff(&self) -> Result<SnarkAccountDiff, DaAccumulationError> {
        let mut seq_builder = DaCounterBuilder::<CtrU64ByU16>::from_source(self.base_seq_no);
        seq_builder.set(self.final_seq_no)?;
        let seq_no = seq_builder.into_write()?;
        let inner_state = DaRegister::compare(
            &self.base_proof_state.inner().inner_state(),
            &self.final_proof_state.inner().inner_state(),
        );
        let mut next_idx_builder = DaCounterBuilder::<CtrU64ByUnsignedVarint>::from_source(
            self.base_proof_state.inner().next_inbox_msg_idx(),
        );
        next_idx_builder.set(self.final_proof_state.inner().next_inbox_msg_idx())?;
        let next_inbox_msg_idx = next_idx_builder.into_write()?;
        let proof_state = DaProofStateDiff::new(inner_state, next_inbox_msg_idx);
        Ok(SnarkAccountDiff::new(
            seq_no,
            proof_state,
            self.inbox.clone(),
        ))
    }
}

/// Tracked account fields for DA diffing.
#[derive(Clone, Debug)]
struct AccountDelta {
    serial: AccountSerial,
    base_balance: BitcoinAmount,
    final_balance: BitcoinAmount,
    ty: AccountTypeId,
    snark: Option<SnarkDelta>,
}

impl AccountDelta {
    fn from_state<T: IAccountState>(state: &T) -> Self {
        let ty = state.ty();
        let snark = match state.type_state() {
            AccountTypeStateRef::Snark(snark_state) => Some(SnarkDelta::from_state(snark_state)),
            AccountTypeStateRef::Empty => None,
        };
        let balance = state.balance();
        Self {
            serial: state.serial(),
            base_balance: balance,
            final_balance: balance,
            ty,
            snark,
        }
    }
}

/// Minimal tracking data for a newly created account.
#[derive(Clone, Debug)]
struct NewAccountRecord {
    serial: AccountSerial,
    account_id: AccountId,
}

/// Per-epoch accumulator of DA writes before encoding.
#[derive(Default)]
struct EpochDaAccumulator {
    /// Slot counter builder for the epoch.
    slot_builder: Option<DaCounterBuilder<CtrU64ByU16>>,

    /// Expected first serial based on the pre-state next_account_serial.
    expected_first_serial: Option<AccountSerial>,

    /// New account records created during the epoch.
    new_account_records: Vec<NewAccountRecord>,

    /// New account IDs for quick lookup.
    new_account_ids: BTreeSet<AccountId>,

    /// Per-account deltas accumulated during the epoch.
    account_deltas: BTreeMap<AccountId, AccountDelta>,
}

impl fmt::Debug for EpochDaAccumulator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EpochDaAccumulator")
            .field("slot_builder_set", &self.slot_builder.is_some())
            .field("expected_first_serial", &self.expected_first_serial)
            .field("new_account_records", &self.new_account_records)
            .field("new_account_ids", &self.new_account_ids)
            .field("account_deltas", &self.account_deltas)
            .finish()
    }
}

impl EpochDaAccumulator {
    /// Records a slot change event.
    fn record_slot_change(&mut self, prior: u64, new: u64) -> Result<(), DaAccumulationError> {
        let builder = self
            .slot_builder
            .get_or_insert_with(|| DaCounterBuilder::<CtrU64ByU16>::from_source(prior));
        builder.set(new)?;
        Ok(())
    }

    /// Ensures there is a tracked delta for an account.
    fn ensure_account_delta<T: IAccountState>(&mut self, account_id: AccountId, state: &T) {
        self.account_deltas
            .entry(account_id)
            .or_insert_with(|| AccountDelta::from_state(state));
    }

    /// Records writes and post-state for an updated account.
    fn record_account_update<S: IStateAccessor, T: IAccountState>(
        &mut self,
        state: &S,
        account_id: AccountId,
        post_state: &T,
        writes: &IndexerWrites,
    ) -> Result<(), DaAccumulationError> {
        let Some(delta) = self.account_deltas.get_mut(&account_id) else {
            return Err(DaAccumulationError::MissingPreState(account_id));
        };

        let actual_ty = post_state.ty();
        if actual_ty != delta.ty {
            return Err(DaAccumulationError::AccountTypeChanged {
                account_id,
                expected: delta.ty,
                actual: actual_ty,
            });
        }

        delta.final_balance = post_state.balance();
        if let (Some(snark_delta), AccountTypeStateRef::Snark(snark_state)) =
            (&mut delta.snark, post_state.type_state())
        {
            snark_delta.update_from_state(snark_state);
        }

        for msg in writes.inbox_messages() {
            if msg.account_id() != account_id {
                return Err(DaAccumulationError::InboxAccountMismatch {
                    expected: account_id,
                    actual: msg.account_id(),
                });
            }
            let Some(snark_delta) = delta.snark.as_mut() else {
                return Err(DaAccumulationError::MissingSnarkState(account_id));
            };
            let entry = msg.entry();
            let payload_len = entry.payload().data().len();
            if payload_len > MAX_MSG_PAYLOAD_BYTES {
                return Err(DaAccumulationError::MessagePayloadTooLarge {
                    provided: payload_len,
                    max: MAX_MSG_PAYLOAD_BYTES,
                });
            }

            let source_id = entry.source();
            if source_id.is_special() {
                return Err(DaAccumulationError::MessageSourceMissing(source_id));
            }
            let exists = state
                .check_account_exists(source_id)
                .map_err(|_| DaAccumulationError::MessageSourceMissing(source_id))?;
            if !exists {
                return Err(DaAccumulationError::MessageSourceMissing(source_id));
            }

            let entry = DaMessageEntry::new(source_id, entry.incl_epoch(), entry.payload().clone());
            if !snark_delta.inbox.append_entry(entry) {
                return Err(DaAccumulationError::InboxBufferFull {
                    account_id,
                    max: InboxBuffer::MAX_INSERT,
                });
            }
        }

        Ok(())
    }

    /// Records a new account.
    fn record_new_account(
        &mut self,
        expected_first_serial: AccountSerial,
        serial: AccountSerial,
        account_id: AccountId,
    ) -> Result<(), DaAccumulationError> {
        if self.expected_first_serial.is_none() {
            self.expected_first_serial = Some(expected_first_serial);
        }
        if !self.new_account_ids.insert(account_id) {
            return Err(DaAccumulationError::DuplicateNewAccountId(account_id));
        }
        self.new_account_records
            .push(NewAccountRecord { serial, account_id });
        Ok(())
    }

    /// Finalizes the epoch by building the state diff.
    fn finalize<S: IStateAccessor>(&mut self, state: &S) -> Result<StateDiff, DaAccumulationError> {
        let global_diff = self.build_global_diff()?;
        let ledger_diff = self.build_ledger_diff(state)?;
        Ok(StateDiff::new(global_diff, ledger_diff))
    }

    /// Builds the global state diff for the epoch.
    fn build_global_diff(&mut self) -> Result<GlobalStateDiff, DaAccumulationError> {
        let cur_slot = if let Some(builder) = self.slot_builder.take() {
            builder.into_write()?
        } else {
            DaCounter::new_unchanged()
        };

        Ok(GlobalStateDiff::new(cur_slot))
    }

    /// Builds the ledger diff for the epoch.
    fn build_ledger_diff<S: IStateAccessor>(
        &self,
        state: &S,
    ) -> Result<LedgerDiff, DaAccumulationError> {
        let mut new_records = self.new_account_records.clone();
        new_records.sort_by_key(|entry| entry.serial);

        if let Some(first) = new_records.first() {
            if let Some(expected) = self.expected_first_serial
                && first.serial != expected
            {
                return Err(DaAccumulationError::NewAccountSerialGap(
                    expected,
                    first.serial,
                ));
            }

            let mut expected = first.serial;
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

        let mut new_account_serials = BTreeSet::new();
        let mut new_accounts = Vec::with_capacity(new_records.len());
        for entry in &new_records {
            if !new_account_serials.insert(entry.serial) {
                return Err(DaAccumulationError::DuplicateAccountSerial(entry.serial));
            }
            let state_ref = state
                .get_account_state(entry.account_id)
                .map_err(|_| DaAccumulationError::MissingAccountState(entry.account_id))?
                .ok_or(DaAccumulationError::MissingAccountState(entry.account_id))?;
            let init = account_init_from_state(state_ref);
            if let AccountTypeInit::Snark(init) = &init.type_state {
                let vk_len = init.update_vk.as_slice().len();
                if vk_len > MAX_VK_BYTES {
                    return Err(DaAccumulationError::VkTooLarge {
                        provided: vk_len,
                        max: MAX_VK_BYTES,
                    });
                }
            }
            new_accounts.push(NewAccountEntry::new(entry.account_id, init));
        }

        let mut account_diffs = Vec::new();
        let mut seen_serials = BTreeSet::new();

        for (account_id, delta) in &self.account_deltas {
            if self.new_account_ids.contains(account_id) {
                continue;
            }

            let balance = DaRegister::compare(&delta.base_balance, &delta.final_balance);
            let snark_state = match delta.ty {
                AccountTypeId::Empty => SnarkAccountDiff::default(),
                AccountTypeId::Snark => {
                    let snark = delta
                        .snark
                        .as_ref()
                        .ok_or(DaAccumulationError::MissingSnarkState(*account_id))?;
                    snark.build_diff()?
                }
            };
            let diff = AccountDiff::new(balance, snark_state);

            if diff.is_default() {
                continue;
            }

            let serial = delta.serial;
            if new_account_serials.contains(&serial) || !seen_serials.insert(serial) {
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
}

/// State accessor that accumulates DA-covered writes for a single epoch.
///
/// This wrapper should only be used for preseal execution; epoch sealing
/// updates derived from L1 must be applied on a non-DA tracking accessor.
#[derive(Debug)]
pub struct DaAccumulatingState<S: IStateAccessor> {
    /// Wrapped state accessor.
    inner: S,

    /// Epoch-scoped DA write accumulator.
    epoch_acc: EpochDaAccumulator,

    /// Pending state diffs waiting for output logs.
    pending_epoch_diffs: VecDeque<StateDiff>,

    /// Completed epoch blobs waiting to be drained.
    pending_epoch_blobs: VecDeque<Vec<u8>>,

    /// Error captured while finalizing an epoch via set_cur_epoch.
    pending_epoch_error: Option<DaAccumulationError>,
    // No log accumulation: OL output logs are posted in checkpoint sidecars.
}

impl<S: IStateAccessor> DaAccumulatingState<S> {
    /// Creates a new DA accumulating state accessor.
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            epoch_acc: EpochDaAccumulator::default(),
            pending_epoch_diffs: VecDeque::new(),
            pending_epoch_blobs: VecDeque::new(),
            pending_epoch_error: None,
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
    pub fn take_completed_epoch_da_blob(&mut self) -> Result<Option<Vec<u8>>, DaAccumulationError> {
        if let Some(err) = self.pending_epoch_error.take() {
            return Err(err);
        }

        if let Some(blob) = self.pending_epoch_blobs.pop_front() {
            return Ok(Some(blob));
        }

        if self.pending_epoch_diffs.front().is_some() {
            let state_diff = self
                .pending_epoch_diffs
                .pop_front()
                .expect("pending diff is available");
            let blob = encode_payload(state_diff)?;
            return Ok(Some(blob));
        }

        let mut acc = take(&mut self.epoch_acc);
        match acc.finalize(&self.inner) {
            Ok(state_diff) => {
                let blob = encode_payload(state_diff)?;
                Ok(Some(blob))
            }
            Err(err) => {
                self.epoch_acc = acc;
                Err(err)
            }
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
        let prior = self.inner.cur_slot();
        if let Err(err) = self.epoch_acc.record_slot_change(prior, slot)
            && self.pending_epoch_error.is_none()
        {
            self.pending_epoch_error = Some(err);
        }
        self.inner.set_cur_slot(slot);
    }

    // ===== Epochal state methods =====

    fn cur_epoch(&self) -> u32 {
        self.inner.cur_epoch()
    }

    fn set_cur_epoch(&mut self, epoch: u32) {
        let prev = self.inner.cur_epoch();
        if epoch != prev {
            let mut acc = take(&mut self.epoch_acc);
            match acc.finalize(&self.inner) {
                Ok(state_diff) => {
                    self.pending_epoch_diffs.push_back(state_diff);
                }
                Err(err) => {
                    if self.pending_epoch_error.is_none() {
                        self.pending_epoch_error = Some(err);
                    }
                }
            }
            self.epoch_acc = EpochDaAccumulator::default();
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
        if let Some(account_state) = self.inner.get_account_state(id)? {
            self.epoch_acc.ensure_account_delta(id, account_state);
        }

        let (result, (local_writes, post_state)) = self.inner.update_account(id, |inner_acct| {
            let mut wrapped = IndexerAccountStateMut::new(inner_acct.clone(), id);
            let user_result = f(&mut wrapped);
            let (modified_inner, writes, was_modified) = wrapped.into_parts();
            if was_modified {
                *inner_acct = modified_inner.clone();
            }
            let post_state = if was_modified {
                Some(modified_inner)
            } else {
                None
            };
            (user_result, (writes, post_state))
        })?;

        if let Some(post_state) = post_state
            && let Err(err) =
                self.epoch_acc
                    .record_account_update(&self.inner, id, &post_state, &local_writes)
            && self.pending_epoch_error.is_none()
        {
            self.pending_epoch_error = Some(err);
        }

        Ok(result)
    }

    fn create_new_account(
        &mut self,
        id: AccountId,
        new_acct_data: NewAccountData<Self::AccountState>,
    ) -> AcctResult<AccountSerial> {
        let expected_first_serial = self.inner.next_account_serial();
        let serial = self.inner.create_new_account(id, new_acct_data)?;

        if let Err(err) = self
            .epoch_acc
            .record_new_account(expected_first_serial, serial, id)
            && self.pending_epoch_error.is_none()
        {
            self.pending_epoch_error = Some(err);
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
}

fn encode_payload(state_diff: StateDiff) -> Result<Vec<u8>, DaAccumulationError> {
    let blob = OLDaPayloadV1::new(state_diff);
    let encoded = encode_to_vec(&blob)?;

    if encoded.len() as u64 > OL_DA_DIFF_MAX_SIZE {
        return Err(DaAccumulationError::PayloadTooLarge {
            provided: encoded.len(),
            max: OL_DA_DIFF_MAX_SIZE,
        });
    }

    Ok(encoded)
}

/// Converts account state into DA init data for encoding.
fn account_init_from_state<T: IAccountState>(state: &T) -> AccountInit {
    let balance = state.balance();
    match state.type_state() {
        AccountTypeStateRef::Empty => AccountInit::new(balance, AccountTypeInit::Empty),
        AccountTypeStateRef::Snark(snark_state) => {
            let init = SnarkAccountInit::new(
                snark_state.inner_state_root(),
                snark_state.update_vk().as_buf_ref().to_bytes(),
            );
            AccountInit::new(balance, AccountTypeInit::Snark(init))
        }
    }
}
