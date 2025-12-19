//! Orchestration layer state write batch.

use std::collections::BTreeMap;

use strata_acct_types::{AccountId, AccountSerial};
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_identifiers::L1BlockCommitment;
use strata_ledger_types::{IAccountStateConstructible, IStateAccessor, NewAccountData};

use crate::{EpochalState, GlobalState, SerialMap};

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

// Manual implementation of Codec for WriteBatch since it has a `BTreeMap<T>` field.
impl<A: Codec> Codec for WriteBatch<A> {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.global.encode(enc)?;
        self.epochal.encode(enc)?;
        self.ledger.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let global = GlobalState::decode(dec)?;
        let epochal = EpochalState::decode(dec)?;
        let ledger = LedgerWriteBatch::decode(dec)?;
        Ok(Self {
            global,
            epochal,
            ledger,
        })
    }
}

impl<A: Codec> Codec for LedgerWriteBatch<A> {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        (self.account_writes.len() as u64).encode(enc)?;
        for (id, state) in &self.account_writes {
            id.encode(enc)?;
            state.encode(enc)?;
        }
        self.serial_to_id.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let account_writes_len = u64::decode(dec)? as usize;
        let mut account_writes = BTreeMap::new();
        for _ in 0..account_writes_len {
            let id = AccountId::decode(dec)?;
            let state = A::decode(dec)?;
            account_writes.insert(id, state);
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
    use strata_acct_types::{AccountId, AccountSerial, BitcoinAmount};
    use strata_codec::{decode_buf_exact, encode_to_vec};
    use strata_identifiers::{Buf32, EpochCommitment, L1BlockCommitment};
    use strata_ledger_types::IAccountState;

    use super::*;
    use crate::account::{NativeAccountState, NativeAccountTypeState};

    fn test_account_id(seed: u8) -> AccountId {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        AccountId::from(bytes)
    }

    fn create_test_global_state() -> GlobalState {
        GlobalState::new(42)
    }

    fn create_test_epochal_state() -> EpochalState {
        let blkid = Buf32::zero().into();
        let l1_block =
            L1BlockCommitment::from_height_u64(100, blkid).expect("valid L1 block commitment");
        let epoch_commitment = EpochCommitment::null();
        EpochalState::new(BitcoinAmount::from_sat(1000), 5, l1_block, epoch_commitment)
    }

    fn create_test_account_state(serial: AccountSerial, balance: u64) -> NativeAccountState {
        NativeAccountState::new(
            serial,
            BitcoinAmount::from_sat(balance),
            NativeAccountTypeState::Empty,
        )
    }

    #[test]
    fn test_serial_map_codec_roundtrip_empty() {
        let map = SerialMap::new();
        let encoded = encode_to_vec(&map).expect("Failed to encode SerialMap");
        let decoded: SerialMap = decode_buf_exact(&encoded).expect("Failed to decode SerialMap");

        assert!(decoded.is_empty());
        assert_eq!(decoded.len(), 0);
    }

    #[test]
    fn test_serial_map_codec_roundtrip_with_entries() {
        let id1 = test_account_id(1);
        let id2 = test_account_id(2);
        let id3 = test_account_id(3);

        let map = SerialMap::new_first(AccountSerial::from(10u32), id1);
        let mut map = map;
        map.insert_next(AccountSerial::from(11u32), id2);
        map.insert_next(AccountSerial::from(12u32), id3);

        let encoded = encode_to_vec(&map).expect("Failed to encode SerialMap");
        let decoded: SerialMap = decode_buf_exact(&encoded).expect("Failed to decode SerialMap");

        assert_eq!(decoded.len(), 3);
        assert_eq!(decoded.first_serial(), Some(AccountSerial::from(10u32)));
        assert_eq!(decoded.last_serial(), Some(AccountSerial::from(12u32)));
        assert_eq!(decoded.get(AccountSerial::from(10u32)), Some(&id1));
        assert_eq!(decoded.get(AccountSerial::from(11u32)), Some(&id2));
        assert_eq!(decoded.get(AccountSerial::from(12u32)), Some(&id3));
    }

    #[test]
    fn test_ledger_write_batch_codec_roundtrip_empty() {
        let batch: LedgerWriteBatch<NativeAccountState> = LedgerWriteBatch::new();
        let encoded = encode_to_vec(&batch).expect("Failed to encode LedgerWriteBatch");
        let decoded: LedgerWriteBatch<NativeAccountState> =
            decode_buf_exact(&encoded).expect("Failed to decode LedgerWriteBatch");

        assert!(decoded.serial_to_id.is_empty());
        assert_eq!(decoded.account_writes.len(), 0);
    }

    #[test]
    fn test_ledger_write_batch_codec_roundtrip_with_accounts() {
        let mut batch: LedgerWriteBatch<NativeAccountState> = LedgerWriteBatch::new();

        let id1 = test_account_id(1);
        let id2 = test_account_id(2);
        let serial1 = AccountSerial::from(100u32);
        let serial2 = AccountSerial::from(101u32);
        let state1 = create_test_account_state(serial1, 1000);
        let state2 = create_test_account_state(serial2, 2000);

        batch.create_account_raw(id1, state1.clone(), serial1);
        batch.create_account_raw(id2, state2.clone(), serial2);

        let encoded = encode_to_vec(&batch).expect("Failed to encode LedgerWriteBatch");
        let decoded: LedgerWriteBatch<NativeAccountState> =
            decode_buf_exact(&encoded).expect("Failed to decode LedgerWriteBatch");

        assert_eq!(decoded.serial_to_id.len(), 2);
        assert_eq!(decoded.account_writes.len(), 2);

        let decoded_state1 = decoded.get_account(&id1).expect("Account 1 not found");
        let decoded_state2 = decoded.get_account(&id2).expect("Account 2 not found");
        assert_eq!(decoded_state1.serial(), state1.serial());
        assert_eq!(decoded_state1.balance(), state1.balance());
        assert_eq!(decoded_state2.serial(), state2.serial());
        assert_eq!(decoded_state2.balance(), state2.balance());
    }

    #[test]
    fn test_write_batch_codec_roundtrip_empty() {
        let global = create_test_global_state();
        let epochal = create_test_epochal_state();
        let batch: WriteBatch<NativeAccountState> =
            WriteBatch::new(global.clone(), epochal.clone());

        let encoded = encode_to_vec(&batch).expect("Failed to encode WriteBatch");
        let decoded: WriteBatch<NativeAccountState> =
            decode_buf_exact(&encoded).expect("Failed to decode WriteBatch");

        assert_eq!(decoded.global().get_cur_slot(), global.get_cur_slot());
        assert_eq!(decoded.epochal().cur_epoch(), epochal.cur_epoch());
        assert!(decoded.ledger().serial_to_id.is_empty());
    }

    #[test]
    fn test_write_batch_codec_roundtrip_with_accounts() {
        let global = create_test_global_state();
        let epochal = create_test_epochal_state();
        let mut batch: WriteBatch<NativeAccountState> = WriteBatch::new(global, epochal);

        let id1 = test_account_id(1);
        let serial1 = AccountSerial::from(100u32);
        let state1 = create_test_account_state(serial1, 5000);

        batch
            .ledger_mut()
            .create_account_raw(id1, state1.clone(), serial1);

        let encoded = encode_to_vec(&batch).expect("Failed to encode WriteBatch");
        let decoded: WriteBatch<NativeAccountState> =
            decode_buf_exact(&encoded).expect("Failed to decode WriteBatch");

        // Verify global state
        assert_eq!(decoded.global().get_cur_slot(), 42);

        // Verify epochal state
        assert_eq!(decoded.epochal().cur_epoch(), 5);

        // Verify ledger
        assert_eq!(decoded.ledger().serial_to_id.len(), 1);
        let decoded_state = decoded
            .ledger()
            .get_account(&id1)
            .expect("Account not found");
        assert_eq!(decoded_state.serial(), state1.serial());
        assert_eq!(decoded_state.balance(), state1.balance());
    }
}
