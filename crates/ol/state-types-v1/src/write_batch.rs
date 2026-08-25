//! Orchestration layer state write batch.

use std::collections::BTreeMap;

use strata_acct_types::{AccountId, AccountSerial, BitcoinAmount, Mmr64};
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_codec_utils::CodecSsz;
use strata_identifiers::{EpochCommitment, L1BlockId, L1Height, Slot};
use strata_ol_state_types::{IAccountState, NewAccountData, PendingAsmLog};

use crate::{OLAccountStateV1, SerialMap};

/// A write to a single ledger account.
///
/// Currently the only supported write is a full replacement of the account
/// state; finer-grained writes can be added later. This is NOT a DA type, so
/// DA considerations do not apply.
#[derive(Clone, Debug)]
pub struct AccountStateWrite(OLAccountStateV1);

impl AccountStateWrite {
    /// Creates an account state write.
    pub fn new(state: OLAccountStateV1) -> Self {
        Self(state)
    }

    /// Returns the replacement account state.
    pub fn state(&self) -> &OLAccountStateV1 {
        &self.0
    }

    /// Returns the replacement account state mutably.
    pub fn state_mut(&mut self) -> &mut OLAccountStateV1 {
        &mut self.0
    }

    /// Consumes the write and returns the replacement account state.
    pub fn into_state(self) -> OLAccountStateV1 {
        self.0
    }

    /// Returns the account serial.
    pub fn serial(&self) -> AccountSerial {
        self.0.serial()
    }
}

/// Tracked writes to the global state.
#[derive(Clone, Debug, Default)]
pub struct GlobalStateWrites {
    /// New slot value, if changed.
    pub cur_slot: Option<Slot>,

    /// New limbo funds value (in satoshis), if changed.
    pub limbo_funds_sats: Option<u64>,
}

/// Tracked writes to the intraepoch state.
#[derive(Clone, Debug, Default)]
pub struct IntraepochStateWrites {
    /// If true, the intraepoch state was reset within this batch. On apply,
    /// the buffer is cleared before any `appended_pending_asm_logs` entries
    /// are appended.
    pub reset: bool,

    /// New pending entries appended during the batch (after the reset, if any).
    pub appended_pending_asm_logs: Vec<PendingAsmLog>,
}

/// Tracked writes to the epochal state.
#[derive(Clone, Debug, Default)]
pub struct EpochalStateWrites {
    /// New epoch number, if changed.
    pub cur_epoch: Option<u32>,

    /// New last L1 block ID, if changed.
    pub last_l1_blkid: Option<L1BlockId>,

    /// New last L1 height, if changed.
    pub last_l1_height: Option<L1Height>,

    /// New ASM recorded epoch, if changed.
    pub asm_recorded_epoch: Option<EpochCommitment>,

    /// New total ledger balance, if changed.
    pub total_ledger_balance: Option<BitcoinAmount>,

    /// New L1 block refs MMR, if changed.
    pub l1_block_refs_mmr: Option<Mmr64>,
}

/// A batch of writes to the OL state.
///
/// This tracks all modifications made during block execution so they can be
/// applied atomically or discarded.
#[derive(Clone, Debug, Default)]
pub struct WriteBatch {
    pub(crate) global_writes: GlobalStateWrites,
    pub(crate) epochal_writes: EpochalStateWrites,
    pub(crate) intraepoch_writes: IntraepochStateWrites,
    pub(crate) ledger: LedgerWriteBatch,
}

impl WriteBatch {
    /// Returns a reference to the global state writes.
    pub fn global_writes(&self) -> &GlobalStateWrites {
        &self.global_writes
    }

    /// Returns a mutable reference to the global state writes.
    pub fn global_writes_mut(&mut self) -> &mut GlobalStateWrites {
        &mut self.global_writes
    }

    /// Returns a reference to the epochal state writes.
    pub fn epochal_writes(&self) -> &EpochalStateWrites {
        &self.epochal_writes
    }

    /// Returns a mutable reference to the epochal state writes.
    pub fn epochal_writes_mut(&mut self) -> &mut EpochalStateWrites {
        &mut self.epochal_writes
    }

    /// Returns a reference to the intraepoch state writes.
    pub fn intraepoch_writes(&self) -> &IntraepochStateWrites {
        &self.intraepoch_writes
    }

    /// Returns a mutable reference to the intraepoch state writes.
    pub fn intraepoch_writes_mut(&mut self) -> &mut IntraepochStateWrites {
        &mut self.intraepoch_writes
    }

    /// Returns a reference to the ledger write batch.
    pub fn ledger(&self) -> &LedgerWriteBatch {
        &self.ledger
    }

    /// Returns a mutable reference to the ledger write batch.
    pub fn ledger_mut(&mut self) -> &mut LedgerWriteBatch {
        &mut self.ledger
    }

    /// Consumes the batch and returns its component parts.
    pub fn into_parts(
        self,
    ) -> (
        GlobalStateWrites,
        EpochalStateWrites,
        IntraepochStateWrites,
        LedgerWriteBatch,
    ) {
        (
            self.global_writes,
            self.epochal_writes,
            self.intraepoch_writes,
            self.ledger,
        )
    }
}

/// Tracks writes to the ledger accounts table.
#[derive(Clone, Debug, Default)]
pub struct LedgerWriteBatch {
    /// Tracks the state of new and updated accounts.
    account_writes: BTreeMap<AccountId, AccountStateWrite>,

    /// Maps serial -> account ID for newly created accounts (contiguous serials).
    serial_to_id: SerialMap,
}

impl LedgerWriteBatch {
    /// Creates a new empty ledger write batch.
    pub fn new() -> Self {
        Self::default()
    }
    /// Records a full-replacement write for a new account with the assigned serial.
    ///
    /// The serial should be obtained from `IStateAccessor::next_account_serial()`.
    pub fn create_account_raw(
        &mut self,
        id: AccountId,
        state: OLAccountStateV1,
        serial: AccountSerial,
    ) {
        #[cfg(debug_assertions)]
        if self.account_writes.contains_key(&id) {
            panic!("state/wb: creating new account at addr that already exists (addr {id})");
        }

        self.account_writes
            .insert(id, AccountStateWrite::new(state));
        let inserted = self.serial_to_id.insert_next(serial, id);
        debug_assert!(
            inserted,
            "state/wb: serial not contiguous (serial {serial})"
        );
    }

    /// Creates a new account from new account data with the given serial.
    ///
    /// The serial should be obtained from `IStateAccessor::next_account_serial()`.
    pub fn create_account_from_data(
        &mut self,
        id: AccountId,
        new_acct_data: NewAccountData,
        serial: AccountSerial,
    ) {
        let state = OLAccountStateV1::new_with_serial(new_acct_data, serial);
        self.create_account_raw(id, state, serial);
    }

    /// Records a full-replacement write for an existing account.
    pub fn update_account(&mut self, id: AccountId, state: OLAccountStateV1) {
        self.account_writes
            .insert(id, AccountStateWrite::new(state));
    }

    /// Gets a written account state, if it exists in the batch.
    pub fn get_account(&self, id: &AccountId) -> Option<&OLAccountStateV1> {
        self.account_writes.get(id).map(AccountStateWrite::state)
    }

    /// Gets a mutable reference to a written account state, if it exists.
    pub fn get_account_mut(&mut self, id: &AccountId) -> Option<&mut OLAccountStateV1> {
        self.account_writes
            .get_mut(id)
            .map(AccountStateWrite::state_mut)
    }

    pub(crate) fn get_account_write(&self, id: &AccountId) -> Option<&AccountStateWrite> {
        self.account_writes.get(id)
    }

    /// Checks if an account exists in the write batch.
    pub fn contains_account(&self, id: &AccountId) -> bool {
        self.account_writes.contains_key(id)
    }

    /// Looks up an account ID by serial in the newly created accounts.
    pub fn find_id_by_serial(&self, serial: AccountSerial) -> Option<AccountId> {
        self.serial_to_id.get(serial).copied()
    }

    /// Returns an iterator over the serials of the new accounts being created.
    pub fn iter_new_accounts(&self) -> impl Iterator<Item = (AccountSerial, &AccountId)> {
        self.serial_to_id.iter()
    }

    /// Returns the list of new account IDs in creation order.
    pub fn new_accounts(&self) -> &[AccountId] {
        self.serial_to_id.ids()
    }

    /// Returns an iterator over all written accounts.
    pub fn iter_accounts(&self) -> impl Iterator<Item = (&AccountId, &OLAccountStateV1)> {
        self.account_writes
            .iter()
            .map(|(id, write)| (id, write.state()))
    }

    pub(crate) fn iter_account_writes(
        &self,
    ) -> impl Iterator<Item = (&AccountId, &AccountStateWrite)> {
        self.account_writes.iter()
    }

    /// Consumes the batch, separating new accounts from updated accounts.
    ///
    /// Returns a tuple of:
    /// - Vector of ([`AccountId`], [`AccountStateWrite`]) for newly created accounts
    ///   (in serial order)
    /// - BTreeMap of remaining account updates (existing accounts only)
    pub fn into_new_and_updated(
        mut self,
    ) -> (
        Vec<(AccountId, AccountStateWrite)>,
        BTreeMap<AccountId, AccountStateWrite>,
    ) {
        let new_account_ids = self.serial_to_id.ids().to_vec();
        let mut new_accounts = Vec::with_capacity(new_account_ids.len());

        for id in new_account_ids {
            // If this is missing the entry for the account then that's fine, we
            // can just skip it.
            if let Some(state) = self.account_writes.remove(&id) {
                new_accounts.push((id, state));
            }
        }

        (new_accounts, self.account_writes)
    }
}

impl Codec for GlobalStateWrites {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        CodecSsz::new(self.cur_slot).encode(enc)?;
        CodecSsz::new(self.limbo_funds_sats).encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        Ok(Self {
            cur_slot: CodecSsz::<Option<Slot>>::decode(dec)?.into_inner(),
            limbo_funds_sats: CodecSsz::<Option<u64>>::decode(dec)?.into_inner(),
        })
    }
}

impl Codec for IntraepochStateWrites {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.reset.encode(enc)?;
        (self.appended_pending_asm_logs.len() as u64).encode(enc)?;
        for entry in &self.appended_pending_asm_logs {
            CodecSsz::new(entry.height()).encode(enc)?;
            CodecSsz::new(entry.log().clone()).encode(enc)?;
        }
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let reset = bool::decode(dec)?;
        let len = u64::decode(dec)? as usize;
        let mut appended_pending_asm_logs = Vec::with_capacity(len);
        for _ in 0..len {
            let height = CodecSsz::<L1Height>::decode(dec)?.into_inner();
            let log = CodecSsz::<strata_asm_manifest_types::AsmLogEntry>::decode(dec)?.into_inner();
            appended_pending_asm_logs.push(PendingAsmLog::new(height, log));
        }
        Ok(Self {
            reset,
            appended_pending_asm_logs,
        })
    }
}

impl Codec for EpochalStateWrites {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        CodecSsz::new(self.cur_epoch).encode(enc)?;
        CodecSsz::new(self.last_l1_blkid).encode(enc)?;
        CodecSsz::new(self.last_l1_height).encode(enc)?;
        CodecSsz::new(self.asm_recorded_epoch).encode(enc)?;
        CodecSsz::new(self.total_ledger_balance).encode(enc)?;
        CodecSsz::new(self.l1_block_refs_mmr.clone()).encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        Ok(Self {
            cur_epoch: CodecSsz::<Option<u32>>::decode(dec)?.into_inner(),
            last_l1_blkid: CodecSsz::<Option<L1BlockId>>::decode(dec)?.into_inner(),
            last_l1_height: CodecSsz::<Option<L1Height>>::decode(dec)?.into_inner(),
            asm_recorded_epoch: CodecSsz::<Option<EpochCommitment>>::decode(dec)?.into_inner(),
            total_ledger_balance: CodecSsz::<Option<BitcoinAmount>>::decode(dec)?.into_inner(),
            l1_block_refs_mmr: CodecSsz::<Option<Mmr64>>::decode(dec)?.into_inner(),
        })
    }
}

impl Codec for AccountStateWrite {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        CodecSsz::new(self.0.clone()).encode(enc)
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let state = CodecSsz::<OLAccountStateV1>::decode(dec)?.into_inner();
        Ok(Self::new(state))
    }
}

impl Codec for WriteBatch {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.global_writes.encode(enc)?;
        self.epochal_writes.encode(enc)?;
        self.intraepoch_writes.encode(enc)?;
        self.ledger.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let global_writes = GlobalStateWrites::decode(dec)?;
        let epochal_writes = EpochalStateWrites::decode(dec)?;
        let intraepoch_writes = IntraepochStateWrites::decode(dec)?;
        let ledger = LedgerWriteBatch::decode(dec)?;
        Ok(Self {
            global_writes,
            epochal_writes,
            intraepoch_writes,
            ledger,
        })
    }
}

// Codec implementation for LedgerWriteBatch
// Uses CodecSsz shim for AccountId and Codec for account writes and SerialMap.
impl Codec for LedgerWriteBatch {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Encode account_writes as a map: length, then (key, value) pairs
        (self.account_writes.len() as u64).encode(enc)?;
        for (id, write) in &self.account_writes {
            CodecSsz::new(*id).encode(enc)?;
            write.encode(enc)?;
        }
        self.serial_to_id.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let len = u64::decode(dec)? as usize;
        let mut account_writes = BTreeMap::new();
        for _ in 0..len {
            let id = CodecSsz::<AccountId>::decode(dec)?.into_inner();
            let write = AccountStateWrite::decode(dec)?;
            account_writes.insert(id, write);
        }
        let serial_to_id = SerialMap::decode(dec)?;
        Ok(Self {
            account_writes,
            serial_to_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use strata_codec::encode_to_vec;

    use super::*;
    use crate::OLAccountTypeStateV1;

    #[test]
    fn account_write_encoding_matches_bare_state_ssz_encoding() {
        let state = OLAccountStateV1::new(
            AccountSerial::from(7u32),
            BitcoinAmount::try_from(1_000u64)
                .expect("amount must not exceed the Bitcoin money supply"),
            OLAccountTypeStateV1::Empty,
        );
        let write = AccountStateWrite::new(state.clone());

        let write_bytes = encode_to_vec(&write).expect("encode account state write");
        let state_bytes =
            encode_to_vec(&CodecSsz::new(state)).expect("encode account state through SSZ shim");

        assert_eq!(write_bytes, state_bytes);
    }
}
