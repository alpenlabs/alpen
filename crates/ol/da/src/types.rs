//! OL DA payload and state diff types.

use std::marker::PhantomData;

use strata_acct_types::{AccountId, BitcoinAmount, Hash, MsgPayload};
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_da_framework::{
    CompoundMember, DaCounter, DaError, DaLinacc, DaQueue, DaQueueTarget, DaRegister, DaWrite,
    LinearAccumulator,
    counter_schemes::{self, CtrU64ByU16},
    make_compound_impl,
};
use strata_identifiers::{AccountSerial, AccountTypeId};
use strata_ledger_types::IStateAccessor;
use strata_ol_chain_types_new::SimpleWithdrawalIntentLogData;
use strata_snark_acct_types::MessageEntry;

/// Maximum size for snark account update VK (64 KiB per SPS-ol-chain-structures and
/// SPS-ol-da-structure).
pub const MAX_VK_BYTES: usize = 64 * 1024;

/// Maximum size for a single message payload (4 KiB per SPS-ol-da-structure).
pub const MAX_MSG_PAYLOAD_BYTES: usize = 4 * 1024;

/// Big-endian u16 wrapper for length encoding.
// NOTE: This greatly decreases the chances of accidentally encoding a u16 as little-endian.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(transparent)]
struct U16Be(u16);

impl Codec for U16Be {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        enc.write_buf(&self.0.to_be_bytes())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let mut buf = [0u8; 2];
        dec.read_buf(&mut buf)?;
        Ok(Self(u16::from_be_bytes(buf)))
    }
}

/// Byte vector encoded with a big-endian u16 length prefix.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct U16LenBytes {
    /// Raw byte payload.
    inner: Vec<u8>,
}

impl U16LenBytes {
    /// Creates a new [`U16LenBytes`] from a byte vector.
    pub fn new(inner: Vec<u8>) -> Self {
        Self { inner }
    }

    /// Returns a slice of the inner byte vector.
    pub fn as_slice(&self) -> &[u8] {
        &self.inner
    }
}

impl Codec for U16LenBytes {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        let len = u16::try_from(self.inner.len()).map_err(|_| CodecError::OverflowContainer)?;
        U16Be(len).encode(enc)?;
        enc.write_buf(&self.inner)
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let len = U16Be::decode(dec)?.0 as usize;
        let mut buf = vec![0u8; len];
        dec.read_buf(&mut buf)?;
        Ok(Self { inner: buf })
    }
}

/// List encoded with a big-endian u16 length prefix.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct U16LenList<T> {
    /// Encoded entries.
    entries: Vec<T>,
}

impl<T> U16LenList<T> {
    /// Creates a new [`U16LenList`] from a vector of entries.
    pub fn new(entries: Vec<T>) -> Self {
        Self { entries }
    }

    /// Returns a slice of the entries.
    pub fn entries(&self) -> &[T] {
        &self.entries
    }

    /// Consumes the list and returns the entries as a vector.
    pub fn into_entries(self) -> Vec<T> {
        self.entries
    }
}

impl<T: Codec> Codec for U16LenList<T> {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        let len = u16::try_from(self.entries.len()).map_err(|_| CodecError::OverflowContainer)?;
        U16Be(len).encode(enc)?;
        for entry in &self.entries {
            entry.encode(enc)?;
        }
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let len = U16Be::decode(dec)?.0 as usize;
        let mut entries = Vec::with_capacity(len);
        for _ in 0..len {
            entries.push(T::decode(dec)?);
        }
        Ok(Self { entries })
    }
}

// ============================================================================
// Top-level DA payload
// ============================================================================

/// Versioned OL DA payload containing the state diff.
#[derive(Debug)]
pub struct OLDaPayloadV1 {
    /// State diff for the epoch.
    pub state_diff: StateDiff,
}

impl OLDaPayloadV1 {
    /// Creates a new [`OLDaPayloadV1`] from a state diff.
    pub fn new(state_diff: StateDiff) -> Self {
        Self { state_diff }
    }
}

impl Codec for OLDaPayloadV1 {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.state_diff.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let state_diff = StateDiff::decode(dec)?;
        Ok(Self { state_diff })
    }
}

// ============================================================================
// State diff structures
// ============================================================================

/// Preseal OL state diff (global + ledger).
#[derive(Debug, Default)]
pub struct StateDiff {
    /// Global state diff.
    pub global: GlobalStateDiff,

    /// Ledger state diff.
    pub ledger: LedgerDiff,
}

impl StateDiff {
    /// Creates a new [`StateDiff`] from a global state diff and ledger diff.
    pub fn new(global: GlobalStateDiff, ledger: LedgerDiff) -> Self {
        Self { global, ledger }
    }
}

impl Codec for StateDiff {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.global.encode(enc)?;
        self.ledger.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let global = GlobalStateDiff::decode(dec)?;
        let ledger = LedgerDiff::decode(dec)?;
        Ok(Self { global, ledger })
    }
}

/// Adapter for applying a state diff to a concrete state accessor.
#[derive(Debug)]
pub struct OLStateDiff<S: IStateAccessor> {
    diff: StateDiff,
    _target: PhantomData<S>,
}

impl<S: IStateAccessor> OLStateDiff<S> {
    pub fn new(diff: StateDiff) -> Self {
        Self {
            diff,
            _target: PhantomData,
        }
    }

    pub fn as_inner(&self) -> &StateDiff {
        &self.diff
    }

    pub fn into_inner(self) -> StateDiff {
        self.diff
    }
}

impl<S: IStateAccessor> Default for OLStateDiff<S> {
    fn default() -> Self {
        Self::new(StateDiff::default())
    }
}

impl<S: IStateAccessor> From<StateDiff> for OLStateDiff<S> {
    fn from(diff: StateDiff) -> Self {
        Self::new(diff)
    }
}

impl<S: IStateAccessor> From<OLStateDiff<S>> for StateDiff {
    fn from(diff: OLStateDiff<S>) -> Self {
        diff.diff
    }
}

impl<S: IStateAccessor> DaWrite for OLStateDiff<S> {
    type Target = S;
    type Context = ();

    fn is_default(&self) -> bool {
        DaWrite::is_default(&self.diff.global) && self.diff.ledger.is_empty()
    }

    fn poll_context(
        &self,
        _target: &Self::Target,
        _context: &Self::Context,
    ) -> Result<(), DaError> {
        if !self.diff.ledger.is_empty() || !DaWrite::is_default(&self.diff.global.pending_withdraws)
        {
            return Err(DaError::InsufficientContext);
        }
        Ok(())
    }

    fn apply(&self, target: &mut Self::Target, _context: &Self::Context) -> Result<(), DaError> {
        if !self.diff.ledger.is_empty() || !DaWrite::is_default(&self.diff.global.pending_withdraws)
        {
            return Err(DaError::InsufficientContext);
        }

        let mut cur_slot = target.cur_slot();
        self.diff.global.cur_slot.apply(&mut cur_slot, &())?;
        target.set_cur_slot(cur_slot);
        Ok(())
    }
}

/// Diff of global state fields covered by DA.
#[derive(Debug)]
pub struct GlobalStateDiff {
    /// Slot counter diff.
    pub cur_slot: DaCounter<CtrU64ByU16>,

    /// Pending withdrawal queue tail diff.
    pub pending_withdraws: DaQueue<PendingWithdrawQueue>,
}

impl Default for GlobalStateDiff {
    fn default() -> Self {
        Self {
            cur_slot: DaCounter::new_unchanged(),
            pending_withdraws: DaQueue::new(),
        }
    }
}

impl GlobalStateDiff {
    /// Creates a new [`GlobalStateDiff`] from a slot counter and pending withdrawal queue.
    pub fn new(
        cur_slot: DaCounter<counter_schemes::CtrU64ByU16>,
        pending_withdraws: DaQueue<PendingWithdrawQueue>,
    ) -> Self {
        Self {
            cur_slot,
            pending_withdraws,
        }
    }
}

make_compound_impl! {
    GlobalStateDiff u8 => GlobalStateTarget {
        cur_slot: counter (counter_schemes::CtrU64ByU16),
        pending_withdraws: compound (DaQueue<PendingWithdrawQueue>),
    }
}

/// Target for applying a global state diff.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GlobalStateTarget {
    /// Current slot value.
    pub cur_slot: u64,

    /// Pending withdrawal queue target.
    pub pending_withdraws: PendingWithdrawQueue,
}

/// Queue target for pending withdrawal intents.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PendingWithdrawQueue {
    /// Queue front index.
    front: u16,

    /// Pending withdrawal entries.
    entries: Vec<SimpleWithdrawalIntentLogData>,
}

impl PendingWithdrawQueue {
    /// Creates a new [`PendingWithdrawQueue`] from a front index and entries.
    pub fn new(front: u16, entries: Vec<SimpleWithdrawalIntentLogData>) -> Self {
        Self { front, entries }
    }

    /// Returns a slice of the entries.
    pub fn entries(&self) -> &[SimpleWithdrawalIntentLogData] {
        &self.entries
    }
}

impl DaQueueTarget for PendingWithdrawQueue {
    type Entry = SimpleWithdrawalIntentLogData;

    fn cur_front(&self) -> u16 {
        self.front
    }

    fn cur_next(&self) -> u16 {
        self.front.saturating_add(self.entries.len() as u16)
    }

    fn increment_front(&mut self, incr: u16) {
        let incr_usize = incr as usize;
        if incr_usize > 0 {
            if incr_usize >= self.entries.len() {
                self.entries.clear();
            } else {
                self.entries.drain(0..incr_usize);
            }
            self.front = self.front.saturating_add(incr);
        }
    }

    fn get(&self, idx: usize) -> Option<&Self::Entry> {
        let idx_u16 = u16::try_from(idx).ok()?;
        if idx_u16 < self.front {
            return None;
        }
        let off = (idx_u16 - self.front) as usize;
        self.entries.get(off)
    }

    fn insert_entries(&mut self, entries: &[Self::Entry]) {
        self.entries.extend_from_slice(entries);
    }
}

/// Diff of ledger state (new accounts + account diffs).
#[derive(Debug)]
pub struct LedgerDiff {
    /// New accounts created during the epoch.
    pub new_accounts: U16LenList<NewAccountEntry>,

    /// Per-account diffs for touched accounts.
    pub account_diffs: U16LenList<AccountDiffEntry>,
}

impl Default for LedgerDiff {
    fn default() -> Self {
        Self {
            new_accounts: U16LenList::new(Vec::new()),
            account_diffs: U16LenList::new(Vec::new()),
        }
    }
}

impl LedgerDiff {
    /// Creates a new [`LedgerDiff`] from a list of new accounts and account diffs.
    pub fn new(
        new_accounts: U16LenList<NewAccountEntry>,
        account_diffs: U16LenList<AccountDiffEntry>,
    ) -> Self {
        Self {
            new_accounts,
            account_diffs,
        }
    }

    /// Returns true when no ledger changes are present.
    pub fn is_empty(&self) -> bool {
        self.new_accounts.entries().is_empty() && self.account_diffs.entries().is_empty()
    }
}

impl Codec for LedgerDiff {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.new_accounts.encode(enc)?;
        self.account_diffs.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let new_accounts = U16LenList::<NewAccountEntry>::decode(dec)?;
        let account_diffs = U16LenList::<AccountDiffEntry>::decode(dec)?;
        Ok(Self {
            new_accounts,
            account_diffs,
        })
    }
}

// ============================================================================
// Ledger diff entries
// ============================================================================

/// New account initialization entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewAccountEntry {
    /// Assigned account serial number.
    pub serial: AccountSerial,

    /// Account identifier.
    pub account_id: AccountId,

    /// Initial account data.
    pub init: AccountInit,
}

impl NewAccountEntry {
    /// Creates a new [`NewAccountEntry`] from a serial, account ID, and initial data.
    pub fn new(serial: AccountSerial, account_id: AccountId, init: AccountInit) -> Self {
        Self {
            serial,
            account_id,
            init,
        }
    }
}

impl Codec for NewAccountEntry {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.serial.encode(enc)?;
        self.account_id.encode(enc)?;
        self.init.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let serial = AccountSerial::decode(dec)?;
        let account_id = AccountId::decode(dec)?;
        let init = AccountInit::decode(dec)?;
        Ok(Self {
            serial,
            account_id,
            init,
        })
    }
}

/// Account initialization data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountInit {
    /// Initial balance for the account.
    pub balance: BitcoinAmount,

    /// Initial type-specific state.
    pub type_state: AccountTypeInit,
}

impl AccountInit {
    /// Creates a new [`AccountInit`] from a balance and type-specific state.
    pub fn new(balance: BitcoinAmount, type_state: AccountTypeInit) -> Self {
        Self {
            balance,
            type_state,
        }
    }

    /// Returns the account type ID.
    pub fn type_id(&self) -> AccountTypeId {
        match self.type_state {
            AccountTypeInit::Empty => AccountTypeId::Empty,
            AccountTypeInit::Snark(_) => AccountTypeId::Snark,
        }
    }
}

impl Codec for AccountInit {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.balance.encode(enc)?;
        let type_id = match self.type_state {
            AccountTypeInit::Empty => 0u8,
            AccountTypeInit::Snark(_) => 1u8,
        };
        type_id.encode(enc)?;
        match &self.type_state {
            AccountTypeInit::Empty => Ok(()),
            AccountTypeInit::Snark(init) => init.encode(enc),
        }
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let balance = BitcoinAmount::decode(dec)?;
        let raw_type_id = u8::decode(dec)?;
        let type_state = match raw_type_id {
            0 => AccountTypeInit::Empty,
            1 => AccountTypeInit::Snark(SnarkAccountInit::decode(dec)?),
            _ => return Err(CodecError::InvalidVariant("account_type_id")),
        };
        Ok(Self {
            balance,
            type_state,
        })
    }
}

/// Type-specific initial state for new accounts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AccountTypeInit {
    /// Empty account with no type state.
    Empty,

    /// Snark account with initial snark state.
    Snark(SnarkAccountInit),
}

/// Snark account initialization data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnarkAccountInit {
    /// Initial inner state root.
    pub initial_state_root: Hash,

    /// Update verification key bytes.
    pub update_vk: U16LenBytes,
}

impl SnarkAccountInit {
    /// Creates a new [`SnarkAccountInit`] from a initial state root and update verification key.
    pub fn new(initial_state_root: Hash, update_vk: Vec<u8>) -> Self {
        Self {
            initial_state_root,
            update_vk: U16LenBytes::new(update_vk),
        }
    }
}

impl Codec for SnarkAccountInit {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.initial_state_root.encode(enc)?;
        self.update_vk.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let initial_state_root = Hash::decode(dec)?;
        let update_vk = U16LenBytes::decode(dec)?;
        if update_vk.as_slice().len() > MAX_VK_BYTES {
            return Err(CodecError::OverflowContainer);
        }
        Ok(Self {
            initial_state_root,
            update_vk,
        })
    }
}

/// Per-account diff entry keyed by account serial.
#[derive(Debug)]
pub struct AccountDiffEntry {
    /// Account serial number.
    pub account_serial: AccountSerial,

    /// Per-account diff.
    pub diff: AccountDiff,
}

impl AccountDiffEntry {
    /// Creates a new [`AccountDiffEntry`] from a serial and diff.
    pub fn new(account_serial: AccountSerial, diff: AccountDiff) -> Self {
        Self {
            account_serial,
            diff,
        }
    }
}

impl Codec for AccountDiffEntry {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.account_serial.encode(enc)?;
        self.diff.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let account_serial = AccountSerial::decode(dec)?;
        let diff = AccountDiff::decode(dec)?;
        Ok(Self {
            account_serial,
            diff,
        })
    }
}

// ============================================================================
// Account diffs
// ============================================================================

/// Per-account diff (balance + snark state).
#[derive(Debug)]
pub struct AccountDiff {
    /// Balance register diff.
    pub balance: DaRegister<BitcoinAmount>,

    /// Snark state diff.
    pub snark_state: SnarkAccountDiff,
}

impl Default for AccountDiff {
    fn default() -> Self {
        Self {
            balance: DaRegister::new_unset(),
            snark_state: <SnarkAccountDiff as Default>::default(),
        }
    }
}

impl AccountDiff {
    /// Creates a new [`AccountDiff`] from a balance and snark state diff.
    pub fn new(balance: DaRegister<BitcoinAmount>, snark_state: SnarkAccountDiff) -> Self {
        Self {
            balance,
            snark_state,
        }
    }
}

make_compound_impl! {
    AccountDiff u8 => AccountDiffTarget {
        balance: register (BitcoinAmount),
        snark_state: compound (SnarkAccountDiff),
    }
}

/// Target for applying an account diff.
#[derive(Debug, Default)]
pub struct AccountDiffTarget {
    /// Current balance value.
    pub balance: BitcoinAmount,

    /// Snark account target.
    pub snark_state: SnarkAccountTarget,
}

/// Diff for snark account state.
#[derive(Debug)]
pub struct SnarkAccountDiff {
    /// Sequence number counter diff.
    pub seq_no: DaCounter<counter_schemes::CtrU64ByU16>,

    /// Proof state register diff.
    pub proof_state: DaRegister<DaProofState>,

    /// Inbox append-only diff.
    pub inbox: DaLinacc<InboxAccumulator>,
}

impl Default for SnarkAccountDiff {
    fn default() -> Self {
        Self {
            seq_no: DaCounter::new_unchanged(),
            proof_state: DaRegister::new_unset(),
            inbox: DaLinacc::new(),
        }
    }
}

impl SnarkAccountDiff {
    /// Creates a new [`SnarkAccountDiff`] from a sequence number, proof state, and inbox MMR.
    pub fn new(
        seq_no: DaCounter<counter_schemes::CtrU64ByU16>,
        proof_state: DaRegister<DaProofState>,
        inbox: DaLinacc<InboxAccumulator>,
    ) -> Self {
        Self {
            seq_no,
            proof_state,
            inbox,
        }
    }
}

make_compound_impl! {
    SnarkAccountDiff u8 => SnarkAccountTarget {
        seq_no: counter (counter_schemes::CtrU64ByU16),
        proof_state: register (DaProofState),
        inbox: compound (DaLinacc<InboxAccumulator>),
    }
}

impl CompoundMember for SnarkAccountDiff {
    fn default() -> Self {
        <SnarkAccountDiff as Default>::default()
    }

    fn is_default(&self) -> bool {
        CompoundMember::is_default(&self.seq_no)
            && CompoundMember::is_default(&self.proof_state)
            && CompoundMember::is_default(&self.inbox)
    }

    fn decode_set(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        Self::decode(dec)
    }

    fn encode_set(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        if CompoundMember::is_default(self) {
            return Err(CodecError::InvalidVariant("snark_account_diff"));
        }
        self.encode(enc)
    }
}

/// Proof state snapshot used in DA diffs.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DaProofState {
    /// Inner state root commitment.
    pub inner_state_root: Hash,

    /// Next message read index.
    pub next_msg_read_idx: u64,
}

impl DaProofState {
    /// Creates a new [`DaProofState`] from a inner state root and next message read index.
    pub fn new(inner_state_root: Hash, next_msg_read_idx: u64) -> Self {
        Self {
            inner_state_root,
            next_msg_read_idx,
        }
    }
}

impl Codec for DaProofState {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.inner_state_root.encode(enc)?;
        self.next_msg_read_idx.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let inner_state_root = Hash::decode(dec)?;
        let next_msg_read_idx = u64::decode(dec)?;
        Ok(Self {
            inner_state_root,
            next_msg_read_idx,
        })
    }
}

/// Target for applying snark account diffs.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SnarkAccountTarget {
    /// Current sequence number.
    pub seq_no: u64,

    /// Current proof state.
    pub proof_state: DaProofState,

    /// Current inbox accumulator.
    pub inbox: InboxAccumulator,
}

// ============================================================================
// Inbox accumulator types
// ============================================================================

/// DA-encoded snark inbox message entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaMessageEntry {
    /// Source account for the message.
    pub source: AccountId,

    /// Epoch in which the message was included.
    pub incl_epoch: u32,

    /// Message payload.
    pub payload: MsgPayload,
}

impl From<MessageEntry> for DaMessageEntry {
    fn from(value: MessageEntry) -> Self {
        Self {
            source: value.source(),
            incl_epoch: value.incl_epoch(),
            payload: value.payload().clone(),
        }
    }
}

impl Codec for DaMessageEntry {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.source.encode(enc)?;
        self.incl_epoch.encode(enc)?;
        self.payload.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let source = AccountId::decode(dec)?;
        let incl_epoch = u32::decode(dec)?;
        let payload = MsgPayload::decode(dec)?;
        if payload.data().len() > MAX_MSG_PAYLOAD_BYTES {
            return Err(CodecError::OverflowContainer);
        }
        Ok(Self {
            source,
            incl_epoch,
            payload,
        })
    }
}

/// Linear accumulator of DA-encoded inbox messages.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InboxAccumulator {
    /// Inbox entries appended during the epoch.
    entries: Vec<DaMessageEntry>,
}

impl InboxAccumulator {
    pub fn entries(&self) -> &[DaMessageEntry] {
        &self.entries
    }
}

impl LinearAccumulator for InboxAccumulator {
    type InsertCnt = u16;
    type EntryData = DaMessageEntry;
    const MAX_INSERT: Self::InsertCnt = u16::MAX;

    fn insert(&mut self, entry: &Self::EntryData) {
        self.entries.push(entry.clone());
    }
}

#[cfg(test)]
mod tests {
    use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload};
    use strata_codec::{decode_buf_exact, encode_to_vec};

    use super::*;

    #[test]
    fn test_da_message_entry_decode_rejects_oversize_payload() {
        let payload = MsgPayload::new(
            BitcoinAmount::from_sat(0),
            vec![0u8; MAX_MSG_PAYLOAD_BYTES + 1],
        );
        let entry = DaMessageEntry {
            source: AccountId::zero(),
            incl_epoch: 0,
            payload,
        };

        let encoded = encode_to_vec(&entry).expect("encode da message entry");
        let decoded: Result<DaMessageEntry, _> = decode_buf_exact(&encoded);
        assert!(decoded.is_err());
    }
}
