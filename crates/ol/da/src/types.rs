//! OL DA blob and state diff types.

use strata_acct_types::{AccountId, BitcoinAmount, Hash, MsgPayload};
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_codec_utils::CodecSsz;
use strata_da_framework::{
    CompoundMember, DaCounter, DaError, DaLinacc, DaQueue, DaQueueTarget, DaRegister, DaWrite,
    LinearAccumulator,
    counter_schemes::{self, CtrU64ByU16},
};
use strata_identifiers::{AccountSerial, AccountTypeId};
use strata_ol_chain_types_new::{OLLog, SimpleWithdrawalIntentLogData};
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

/// Versioned OL DA payload containing state diff and withdrawal intents.
#[derive(Debug)]
pub struct OLDaBlobV1 {
    /// State diff for the epoch.
    pub state_diff: StateDiff,
}

impl OLDaBlobV1 {
    /// Creates a new [`OlDaBlobV1`] from a state diff, withdrawal intents, and output logs.
    pub fn new(state_diff: StateDiff) -> Self {
        Self { state_diff }
    }
}

impl Codec for OLDaBlobV1 {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.state_diff.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let state_diff = StateDiff::decode(dec)?;
        Ok(Self { state_diff })
    }
}

/// Withdrawal intents included in the DA payload.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WithdrawalIntents {
    /// Collected intents for the epoch.
    entries: Vec<SimpleWithdrawalIntentLogData>,
}

/// Output logs included in the DA payload.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OutputLogs {
    /// Ordered log entries for the epoch.
    entries: Vec<OLLog>,
}

impl OutputLogs {
    /// Creates a new [`OutputLogs`] from a vector of logs.
    pub fn new(entries: Vec<OLLog>) -> Self {
        Self { entries }
    }

    /// Returns a slice of the entries.
    pub fn entries(&self) -> &[OLLog] {
        &self.entries
    }
}

impl Codec for OutputLogs {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        let len = u16::try_from(self.entries.len()).map_err(|_| CodecError::OverflowContainer)?;
        U16Be(len).encode(enc)?;
        for entry in &self.entries {
            CodecSsz::new(entry.clone()).encode(enc)?;
        }
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let len = U16Be::decode(dec)?.0 as usize;
        let mut entries = Vec::with_capacity(len);
        for _ in 0..len {
            let entry = CodecSsz::<OLLog>::decode(dec)?.into_inner();
            entries.push(entry);
        }
        Ok(Self { entries })
    }
}

impl WithdrawalIntents {
    /// Creates a new [`WithdrawalIntents`] from a vector of withdrawal intents.
    pub fn new(entries: Vec<SimpleWithdrawalIntentLogData>) -> Self {
        Self { entries }
    }

    /// Returns a slice of the entries.
    pub fn entries(&self) -> &[SimpleWithdrawalIntentLogData] {
        &self.entries
    }
}

impl Codec for WithdrawalIntents {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        U16LenList::new(self.entries.clone()).encode(enc)
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let entries = U16LenList::<SimpleWithdrawalIntentLogData>::decode(dec)?.into_entries();
        Ok(Self { entries })
    }
}

// ============================================================================
// State diff structures
// ============================================================================

/// Preseal OL state diff (global + ledger).
#[derive(Debug)]
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

impl Codec for GlobalStateDiff {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Build a presence bitmask to keep encoding deterministic and compact.
        let mut mask: u8 = 0;
        // Emit the bitmask before any optional fields.
        if !CompoundMember::is_default(&self.cur_slot) {
            mask |= 1 << 0;
        }
        // Encode the withdrawal queue tail diff only when present.
        if !CompoundMember::is_default(&self.pending_withdraws) {
            mask |= 1 << 1;
        }
        mask.encode(enc)?;
        // Encode the slot diff only when present.
        if (mask & (1 << 0)) != 0 {
            self.cur_slot.encode_set(enc)?;
        }
        // Encode the withdrawal queue tail diff only when present.
        if (mask & (1 << 1)) != 0 {
            self.pending_withdraws.encode_set(enc)?;
        }
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        // Decode the presence bitmask first.
        let mask = u8::decode(dec)?;
        // Decode the slot diff only when present.
        let cur_slot = if (mask & (1 << 0)) != 0 {
            <DaCounter<counter_schemes::CtrU64ByU16> as CompoundMember>::decode_set(dec)?
        } else {
            DaCounter::new_unchanged()
        };
        // Decode the withdrawal queue tail diff only when present.
        let pending_withdraws = if (mask & (1 << 1)) != 0 {
            <DaQueue<PendingWithdrawQueue> as CompoundMember>::decode_set(dec)?
        } else {
            DaQueue::new()
        };
        // Return the new global state diff.
        Ok(Self {
            cur_slot,
            pending_withdraws,
        })
    }
}

impl DaWrite for GlobalStateDiff {
    type Target = GlobalStateTarget;
    type Context = ();

    fn is_default(&self) -> bool {
        CompoundMember::is_default(&self.cur_slot)
            && CompoundMember::is_default(&self.pending_withdraws)
    }

    fn apply(&self, target: &mut Self::Target, context: &Self::Context) -> Result<(), DaError> {
        self.cur_slot.apply(&mut target.cur_slot, context)?;
        self.pending_withdraws
            .apply(&mut target.pending_withdraws, context)?;
        Ok(())
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

impl Codec for AccountDiff {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        let mut mask: u8 = 0;
        if !CompoundMember::is_default(&self.balance) {
            mask |= 1 << 0;
        }
        if !CompoundMember::is_default(&self.snark_state) {
            mask |= 1 << 1;
        }
        mask.encode(enc)?;
        if (mask & (1 << 0)) != 0 {
            self.balance.encode_set(enc)?;
        }
        if (mask & (1 << 1)) != 0 {
            self.snark_state.encode_set(enc)?;
        }
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let mask = u8::decode(dec)?;
        let balance = if (mask & (1 << 0)) != 0 {
            <DaRegister<BitcoinAmount> as CompoundMember>::decode_set(dec)?
        } else {
            DaRegister::new_unset()
        };
        let snark_state = if (mask & (1 << 1)) != 0 {
            SnarkAccountDiff::decode(dec)?
        } else {
            <SnarkAccountDiff as Default>::default()
        };
        Ok(Self {
            balance,
            snark_state,
        })
    }
}

impl DaWrite for AccountDiff {
    type Target = AccountDiffTarget;
    type Context = ();

    fn is_default(&self) -> bool {
        CompoundMember::is_default(&self.balance) && CompoundMember::is_default(&self.snark_state)
    }

    fn apply(&self, target: &mut Self::Target, context: &Self::Context) -> Result<(), DaError> {
        self.balance.apply(&mut target.balance, context)?;
        self.snark_state.apply(&mut target.snark, context)?;
        Ok(())
    }
}

/// Target for applying an account diff.
#[derive(Debug, Default)]
pub struct AccountDiffTarget {
    /// Current balance value.
    pub balance: BitcoinAmount,

    /// Snark account target.
    pub snark: SnarkAccountTarget,
}

/// Diff for snark account state.
#[derive(Debug)]
pub struct SnarkAccountDiff {
    /// Sequence number counter diff.
    pub seq_no: DaCounter<counter_schemes::CtrU64ByU16>,

    /// Proof state register diff.
    pub proof_state: DaRegister<DaProofState>,

    /// Inbox MMR append-only diff.
    pub inbox_mmr: DaLinacc<InboxAccumulator>,
}

impl Default for SnarkAccountDiff {
    fn default() -> Self {
        Self {
            seq_no: DaCounter::new_unchanged(),
            proof_state: DaRegister::new_unset(),
            inbox_mmr: DaLinacc::new(),
        }
    }
}

impl SnarkAccountDiff {
    /// Creates a new [`SnarkAccountDiff`] from a sequence number, proof state, and inbox MMR.
    pub fn new(
        seq_no: DaCounter<counter_schemes::CtrU64ByU16>,
        proof_state: DaRegister<DaProofState>,
        inbox_mmr: DaLinacc<InboxAccumulator>,
    ) -> Self {
        Self {
            seq_no,
            proof_state,
            inbox_mmr,
        }
    }
}

impl Codec for SnarkAccountDiff {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Build a presence bitmask to keep encoding deterministic and compact.
        let mut mask: u8 = 0;
        // Emit the bitmask before any optional fields.
        if !CompoundMember::is_default(&self.seq_no) {
            mask |= 1 << 0;
        }
        // Encode the proof state diff only when present.
        if !CompoundMember::is_default(&self.proof_state) {
            mask |= 1 << 1;
        }
        // Encode the inbox MMR diff only when present.
        if !CompoundMember::is_default(&self.inbox_mmr) {
            mask |= 1 << 2;
        }
        mask.encode(enc)?;
        // Encode the sequence number diff only when present.
        if (mask & (1 << 0)) != 0 {
            self.seq_no.encode_set(enc)?;
        }
        // Encode the proof state diff only when present.
        if (mask & (1 << 1)) != 0 {
            self.proof_state.encode_set(enc)?;
        }
        // Encode the inbox MMR diff only when present.
        if (mask & (1 << 2)) != 0 {
            self.inbox_mmr.encode_set(enc)?;
        }
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        // Decode the presence bitmask first.
        let mask = u8::decode(dec)?;
        // Decode the sequence number diff only when present.
        let seq_no = if (mask & (1 << 0)) != 0 {
            <DaCounter<counter_schemes::CtrU64ByU16> as CompoundMember>::decode_set(dec)?
        } else {
            DaCounter::new_unchanged()
        };
        // Decode the proof state diff only when present.
        let proof_state = if (mask & (1 << 1)) != 0 {
            <DaRegister<DaProofState> as CompoundMember>::decode_set(dec)?
        } else {
            DaRegister::new_unset()
        };
        // Decode the inbox MMR diff only when present.
        let inbox_mmr = if (mask & (1 << 2)) != 0 {
            <DaLinacc<InboxAccumulator> as CompoundMember>::decode_set(dec)?
        } else {
            DaLinacc::new()
        };
        // Return the new snark account diff.
        Ok(Self {
            seq_no,
            proof_state,
            inbox_mmr,
        })
    }
}

impl CompoundMember for SnarkAccountDiff {
    fn default() -> Self {
        <SnarkAccountDiff as Default>::default()
    }

    fn is_default(&self) -> bool {
        CompoundMember::is_default(&self.seq_no)
            && CompoundMember::is_default(&self.proof_state)
            && CompoundMember::is_default(&self.inbox_mmr)
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

impl DaWrite for SnarkAccountDiff {
    type Target = SnarkAccountTarget;
    type Context = ();

    fn is_default(&self) -> bool {
        CompoundMember::is_default(&self.seq_no)
            && CompoundMember::is_default(&self.proof_state)
            && CompoundMember::is_default(&self.inbox_mmr)
    }

    fn apply(&self, target: &mut Self::Target, context: &Self::Context) -> Result<(), DaError> {
        self.seq_no.apply(&mut target.seq_no, context)?;
        self.proof_state.apply(&mut target.proof_state, context)?;
        self.inbox_mmr.apply(&mut target.inbox, context)?;
        Ok(())
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
