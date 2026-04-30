//! Orchestration layer state write batch.

use std::collections::BTreeMap;

use ssz::{Decode, Encode};
use strata_acct_types::{AccountId, AccountSerial, BitcoinAmount, Mmr64};
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_codec_utils::CodecSsz;
use strata_identifiers::{EpochCommitment, L1BlockId, L1Height, Slot};
use strata_ledger_types::{IAccountState, NewAccountData};

use crate::SerialMap;

/// Tracked writes to the global state.
#[derive(Clone, Debug, Default)]
pub struct GlobalStateWrites {
    /// New slot value, if changed.
    pub cur_slot: Option<Slot>,
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

    /// New ASM manifests MMR, if changed.
    pub asm_manifests_mmr: Option<Mmr64>,
}

/// A batch of writes to the OL state.
///
/// This tracks all modifications made during block execution so they can be
/// applied atomically or discarded.
#[derive(Clone, Debug)]
pub struct WriteBatch<A> {
    pub(crate) global_writes: GlobalStateWrites,
    pub(crate) epochal_writes: EpochalStateWrites,
    pub(crate) ledger: LedgerWriteBatch<A>,
}

impl<A> Default for WriteBatch<A> {
    fn default() -> Self {
        Self {
            global_writes: GlobalStateWrites::default(),
            epochal_writes: EpochalStateWrites::default(),
            ledger: LedgerWriteBatch::new(),
        }
    }
}

impl<A> WriteBatch<A> {
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

    /// Returns a reference to the ledger write batch.
    pub fn ledger(&self) -> &LedgerWriteBatch<A> {
        &self.ledger
    }

    /// Returns a mutable reference to the ledger write batch.
    pub fn ledger_mut(&mut self) -> &mut LedgerWriteBatch<A> {
        &mut self.ledger
    }

    /// Consumes the batch and returns its component parts.
    pub fn into_parts(self) -> (GlobalStateWrites, EpochalStateWrites, LedgerWriteBatch<A>) {
        (self.global_writes, self.epochal_writes, self.ledger)
    }
}

/// Tracks writes to the ledger accounts table.
#[derive(Clone, Debug)]
pub struct LedgerWriteBatch<A> {
    /// Tracks the state of new and updated accounts.
    account_writes: BTreeMap<AccountId, A>,

    /// Maps serial -> account ID for newly created accounts (contiguous serials).
    serial_to_id: SerialMap,
}

impl<A> LedgerWriteBatch<A> {
    /// Creates a new empty ledger write batch.
    pub fn new() -> Self {
        Self::default()
    }
}

impl<A: IAccountState> LedgerWriteBatch<A> {
    /// Tracks creating a new account with the given pre-built state and assigned serial.
    ///
    /// The serial should be obtained from `IStateAccessor::next_account_serial()`.
    pub fn create_account_raw(&mut self, id: AccountId, state: A, serial: AccountSerial) {
        #[cfg(debug_assertions)]
        if self.account_writes.contains_key(&id) {
            panic!("state/wb: creating new account at addr that already exists (addr {id})");
        }

        self.account_writes.insert(id, state);
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
        let state = A::new_with_serial(new_acct_data, serial);
        self.create_account_raw(id, state, serial);
    }

    /// Tracks an update to an existing account.
    pub fn update_account(&mut self, id: AccountId, state: A) {
        self.account_writes.insert(id, state);
    }

    /// Gets a written account state, if it exists in the batch.
    pub fn get_account(&self, id: &AccountId) -> Option<&A> {
        self.account_writes.get(id)
    }

    /// Gets a mutable reference to a written account state, if it exists.
    pub fn get_account_mut(&mut self, id: &AccountId) -> Option<&mut A> {
        self.account_writes.get_mut(id)
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
    pub fn iter_accounts(&self) -> impl Iterator<Item = (&AccountId, &A)> {
        self.account_writes.iter()
    }

    /// Consumes the batch, separating new accounts from updated accounts.
    ///
    /// Returns a tuple of:
    /// - Iterator over (AccountId, A) for newly created accounts (in serial order)
    /// - BTreeMap of remaining account updates (existing accounts only)
    pub fn into_new_and_updated(mut self) -> (Vec<(AccountId, A)>, BTreeMap<AccountId, A>) {
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

impl<A> Default for LedgerWriteBatch<A> {
    fn default() -> Self {
        Self {
            account_writes: BTreeMap::new(),
            serial_to_id: SerialMap::new(),
        }
    }
}

impl Codec for GlobalStateWrites {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        CodecSsz::new(self.cur_slot).encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        Ok(Self {
            cur_slot: CodecSsz::<Option<Slot>>::decode(dec)?.into_inner(),
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
        CodecSsz::new(self.asm_manifests_mmr.clone()).encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        Ok(Self {
            cur_epoch: CodecSsz::<Option<u32>>::decode(dec)?.into_inner(),
            last_l1_blkid: CodecSsz::<Option<L1BlockId>>::decode(dec)?.into_inner(),
            last_l1_height: CodecSsz::<Option<L1Height>>::decode(dec)?.into_inner(),
            asm_recorded_epoch: CodecSsz::<Option<EpochCommitment>>::decode(dec)?.into_inner(),
            total_ledger_balance: CodecSsz::<Option<BitcoinAmount>>::decode(dec)?.into_inner(),
            asm_manifests_mmr: CodecSsz::<Option<Mmr64>>::decode(dec)?.into_inner(),
        })
    }
}

impl<A: Encode + Decode + Clone> Codec for WriteBatch<A> {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.global_writes.encode(enc)?;
        self.epochal_writes.encode(enc)?;
        self.ledger.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let global_writes = GlobalStateWrites::decode(dec)?;
        let epochal_writes = EpochalStateWrites::decode(dec)?;
        let ledger = LedgerWriteBatch::decode(dec)?;
        Ok(Self {
            global_writes,
            epochal_writes,
            ledger,
        })
    }
}

// Codec implementation for LedgerWriteBatch
// Uses CodecSsz shim for SSZ types (AccountId, A)
// and Codec for non-SSZ types (SerialMap)
impl<A: Encode + Decode + Clone> Codec for LedgerWriteBatch<A> {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Encode account_writes as a map: length, then (key, value) pairs
        (self.account_writes.len() as u64).encode(enc)?;
        for (id, state) in &self.account_writes {
            CodecSsz::new(*id).encode(enc)?;
            CodecSsz::new(state.clone()).encode(enc)?;
        }
        self.serial_to_id.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let len = u64::decode(dec)? as usize;
        let mut account_writes = BTreeMap::new();
        for _ in 0..len {
            let id = CodecSsz::<AccountId>::decode(dec)?.into_inner();
            let state = CodecSsz::<A>::decode(dec)?.into_inner();
            account_writes.insert(id, state);
        }
        let serial_to_id = SerialMap::decode(dec)?;
        Ok(Self {
            account_writes,
            serial_to_id,
        })
    }
}
