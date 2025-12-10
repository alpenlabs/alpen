//! Orchestration layer state write batch.

use std::collections::BTreeMap;

use strata_acct_types::{AccountId, AccountSerial, BitcoinAmount};
use strata_ledger_types::{IAccountStateConstructible, NewAccountData};
use strata_ol_state_types::{EpochalState, GlobalState};

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
}

/// Tracks writes to the ledger accounts table.
#[derive(Clone, Debug)]
pub struct LedgerWriteBatch<A> {
    /// Tracks the state of new and updated accounts.
    account_writes: BTreeMap<AccountId, A>,

    /// Tracks the order we insert new accounts into the serials MMR.
    new_accounts: Vec<AccountId>,

    /// Maps serial -> account ID for newly created accounts.
    serial_to_id: BTreeMap<AccountSerial, AccountId>,

    /// The next serial to assign to a new account.
    next_serial: Option<AccountSerial>,
}

impl<A> Default for LedgerWriteBatch<A> {
    fn default() -> Self {
        Self {
            account_writes: BTreeMap::new(),
            new_accounts: Vec::new(),
            serial_to_id: BTreeMap::new(),
            next_serial: None,
        }
    }
}

impl<A> LedgerWriteBatch<A> {
    /// Creates a new empty ledger write batch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the next serial value (should be called during initialization).
    pub fn set_next_serial(&mut self, serial: AccountSerial) {
        self.next_serial = Some(serial);
    }

    /// Tracks creating a new account with the given pre-built state.
    ///
    /// This is the low-level method that accepts an already-constructed account state.
    /// Use `create_account_from_parts` for a higher-level interface.
    pub fn create_account_raw(&mut self, id: AccountId, state: A) -> AccountSerial {
        #[cfg(debug_assertions)]
        if self.account_writes.contains_key(&id) {
            panic!("state/wb: creating new account at addr that already exists");
        }

        let serial = self
            .next_serial
            .expect("state/wb: next_serial not initialized");

        self.account_writes.insert(id, state);
        self.new_accounts.push(id);
        self.serial_to_id.insert(serial, id);

        // Increment next serial
        let raw: u32 = serial.into();
        self.next_serial = Some(AccountSerial::from(raw + 1));

        serial
    }

    /// Creates a new account from new account data, assigning a serial automatically.
    ///
    /// This is the preferred method for creating accounts as it handles serial
    /// assignment internally using `IAccountStateConstructible`.
    pub fn create_account_from_data(
        &mut self,
        id: AccountId,
        new_acct_data: NewAccountData<A>,
    ) -> AccountSerial
    where
        A: IAccountStateConstructible,
    {
        let serial = self
            .next_serial
            .expect("state/wb: next_serial not initialized");

        let state = A::new_with_serial(new_acct_data, serial);
        self.create_account_raw(id, state)
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
        self.serial_to_id.get(&serial).copied()
    }

    /// Returns the list of new account IDs in creation order.
    pub fn new_accounts(&self) -> &[AccountId] {
        &self.new_accounts
    }

    /// Returns an iterator over all written accounts.
    pub fn iter_accounts(&self) -> impl Iterator<Item = (&AccountId, &A)> {
        self.account_writes.iter()
    }
}
