//! Orchestration layer state write batch.

use std::collections::BTreeMap;

use ssz::{Decode, Encode};
use strata_acct_types::{AccountId, AccountSerial};
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_codec_utils::CodecSsz;
use strata_identifiers::L1BlockCommitment;
use strata_ledger_types::{IAccountStateConstructible, IStateAccessor, NewAccountData};

use crate::{
    SerialMap,
    ssz_generated::ssz::state::{EpochalState, GlobalState},
};

/// A batch of writes to the OL state.
///
/// This tracks all modifications made during block execution so they can be
/// applied atomically or discarded.
#[derive(Clone, Debug)]
pub struct WriteBatch<A> {
    pub(crate) global: GlobalState,
    pub(crate) epochal: EpochalState,
    pub(crate) ledger: LedgerWriteBatch<A>,
}

impl<A> WriteBatch<A> {
    /// Creates a new write batch initialized from the given state components.
    pub fn new(global: GlobalState, epochal: EpochalState) -> Self {
        Self {
            global,
            epochal,
            ledger: LedgerWriteBatch::new(),
        }
    }

    /// Creates a new write batch by extracting state from a state accessor.
    ///
    /// This initializes the global and epochal state from the accessor's current values.
    pub fn new_from_state<S>(state: &S) -> Self
    where
        S: IStateAccessor<AccountState = A>,
    {
        // TODO provide accessors/constructors to simplify this
        let global = GlobalState::new(state.cur_slot());
        let epochal = EpochalState::new(
            state.total_ledger_balance(),
            state.cur_epoch(),
            L1BlockCommitment::from_height_u64(
                state.last_l1_height() as u64,
                *state.last_l1_blkid(),
            )
            .expect("state: invalid L1 height"),
            *state.asm_recorded_epoch(),
            state.asm_manifests_mmr().clone(),
        );
        WriteBatch::new(global, epochal)
    }

    /// Returns a reference to the global state in this batch.
    pub fn global(&self) -> &GlobalState {
        &self.global
    }

    /// Returns a mutable reference to the global state in this batch.
    pub fn global_mut(&mut self) -> &mut GlobalState {
        &mut self.global
    }

    /// Returns a reference to the epochal state in this batch.
    pub fn epochal(&self) -> &EpochalState {
        &self.epochal
    }

    /// Returns a mutable reference to the epochal state in this batch.
    pub fn epochal_mut(&mut self) -> &mut EpochalState {
        &mut self.epochal
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
    pub fn into_parts(self) -> (GlobalState, EpochalState, LedgerWriteBatch<A>) {
        (self.global, self.epochal, self.ledger)
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

    /// Tracks creating a new account with the given pre-built state and assigned serial.
    ///
    /// The serial should be obtained from `IStateAccessor::next_account_serial()`.
    pub fn create_account_raw(&mut self, id: AccountId, state: A, serial: AccountSerial) {
        #[cfg(debug_assertions)]
        if self.account_writes.contains_key(&id) {
            panic!("state/wb: creating new account at addr that already exists");
        }

        self.account_writes.insert(id, state);
        let inserted = self.serial_to_id.insert_next(serial, id);
        debug_assert!(inserted, "state/wb: serial not contiguous");
    }

    /// Creates a new account from new account data with the given serial.
    ///
    /// The serial should be obtained from `IStateAccessor::next_account_serial()`.
    pub fn create_account_from_data(
        &mut self,
        id: AccountId,
        new_acct_data: NewAccountData<A>,
        serial: AccountSerial,
    ) where
        A: IAccountStateConstructible,
    {
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

// Codec implementation for WriteBatch - needed for database serialization
// Uses CodecSsz shim for SSZ types (GlobalState, EpochalState)
// and Codec for non-SSZ types (LedgerWriteBatch)
impl<A: Encode + Decode + Clone> Codec for WriteBatch<A> {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        CodecSsz::new(self.global.clone()).encode(enc)?;
        CodecSsz::new(self.epochal.clone()).encode(enc)?;
        self.ledger.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let global = CodecSsz::<GlobalState>::decode(dec)?.into_inner();
        let epochal = CodecSsz::<EpochalState>::decode(dec)?.into_inner();
        let ledger = LedgerWriteBatch::decode(dec)?;
        Ok(Self {
            global,
            epochal,
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
