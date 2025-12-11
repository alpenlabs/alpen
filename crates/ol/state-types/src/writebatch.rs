use std::{collections::HashMap, fmt};

use strata_acct_types::{AccountId, AccountSerial};
use strata_ledger_types::{AsmManifest, StateAccessor};
use strata_snark_acct_types::MessageEntry;

/// Copy-on-Write overlay for state modifications during block execution.
///
/// This structure acts as a sparse, in-memory layer on top of a base state
/// (which could be DB-backed). Reads check the overlay first, then fall through
/// to the base. Writes populate the overlay.
#[derive(Clone)]
pub struct WriteBatch<S: StateAccessor> {
    /// Accounts modified during execution (CoW overlay).
    /// Presence in this map indicates the account was accessed mutably.
    pub(crate) modified_accounts: HashMap<AccountId, S::AccountState>,

    /// Global state override. None means use base state.
    pub(crate) global_state: Option<S::GlobalState>,

    /// Epochal state override. None means use base state.
    pub(crate) epochal_state: Option<S::L1ViewState>,

    /// Next available serial number.
    /// Initialized from base state and incremented as new accounts are created.
    pub(crate) next_serial: AccountSerial,

    /// IDs of newly created accounts, tracked in order of creation.
    /// We only store IDs here; the actual state is in modified_accounts.
    /// This maintains creation order for serial assignment during apply.
    pub(crate) created_ids: Vec<AccountId>,

    /// Mapping of newly assigned serials to account IDs created during execution.
    /// This is needed for find_account_id_by_serial to work correctly during execution.
    pub(crate) serial_to_id: HashMap<AccountSerial, AccountId>,
}

impl<S: StateAccessor> fmt::Debug for WriteBatch<S>
where
    S::GlobalState: fmt::Debug,
    S::L1ViewState: fmt::Debug,
    S::AccountState: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WriteBatch")
            .field("modified_accounts", &self.modified_accounts)
            .field("global_state", &self.global_state)
            .field("epochal_state", &self.epochal_state)
            .field("next_serial", &self.next_serial)
            .field("created_ids", &self.created_ids)
            .field("serial_to_id", &self.serial_to_id)
            .finish()
    }
}

impl<S: StateAccessor> WriteBatch<S> {
    /// Create a new empty WriteBatch with the initial serial number from base state
    pub fn new(next_serial: AccountSerial) -> Self {
        Self {
            modified_accounts: Default::default(),
            global_state: None,
            epochal_state: None,
            next_serial,
            created_ids: Vec::new(),
            serial_to_id: HashMap::new(),
        }
    }

    /// Check if an account exists in the overlay
    pub fn has_account(&self, id: &AccountId) -> bool {
        self.modified_accounts.contains_key(id)
    }

    /// Get a reference to a modified account, if it exists in the overlay
    pub fn get_account(&self, id: &AccountId) -> Option<&S::AccountState> {
        self.modified_accounts.get(id)
    }

    /// Get a mutable reference to a modified account, if it exists in the overlay
    pub fn get_account_mut(&mut self, id: &AccountId) -> Option<&mut S::AccountState> {
        self.modified_accounts.get_mut(id)
    }

    /// Insert or update an account in the overlay
    pub fn insert_account(&mut self, id: AccountId, state: S::AccountState) {
        self.modified_accounts.insert(id, state);
    }

    /// Get global state if overridden
    pub fn global_state(&self) -> Option<&S::GlobalState> {
        self.global_state.as_ref()
    }

    /// Get mutable global state, creating if needed
    pub fn global_state_mut_or_insert(&mut self, base: &S::GlobalState) -> &mut S::GlobalState
    where
        S::GlobalState: Clone,
    {
        self.global_state.get_or_insert_with(|| base.clone())
    }

    /// Get epochal state if overridden
    pub fn epochal_state(&self) -> Option<&S::L1ViewState> {
        self.epochal_state.as_ref()
    }

    /// Get mutable epochal state, creating if needed
    pub fn epochal_state_mut_or_insert(&mut self, base: &S::L1ViewState) -> &mut S::L1ViewState
    where
        S::L1ViewState: Clone,
    {
        self.epochal_state.get_or_insert_with(|| base.clone())
    }

    /// Get the number of modified accounts
    pub fn modified_accounts_count(&self) -> usize {
        self.modified_accounts.len()
    }

    /// Get the next available serial number
    pub fn get_next_serial(&self) -> AccountSerial {
        self.next_serial
    }

    /// Create a new account in the overlay, assigning it the next serial.
    /// Returns the assigned serial.
    pub fn create_account(&mut self, id: AccountId, state: S::AccountState) -> AccountSerial {
        let serial = self.next_serial;

        // Track the serial-to-id mapping
        self.serial_to_id.insert(serial, id);

        // Track the ID in creation order
        self.created_ids.push(id);

        // Store the state in modified_accounts (this is the single source of truth)
        self.modified_accounts.insert(id, state);

        // Increment for next account
        self.next_serial = AccountSerial::from(u32::from(self.next_serial) + 1);

        serial
    }

    /// Find account ID by serial, checking overlay first
    pub fn find_serial(&self, serial: AccountSerial) -> Option<AccountId> {
        self.serial_to_id.get(&serial).copied()
    }
}

/// Auxiliary data for database persistence (not part of consensus state root)
#[derive(Clone, Debug, Default)]
pub struct ExecutionAuxiliaryData {
    /// Messages added to each account's inbox during this block
    /// Stored separately for DB indexing and queries
    pub account_message_additions: HashMap<AccountId, Vec<MessageEntry>>,

    /// ASM manifests processed during this block
    /// Stored separately for DB indexing
    // TODO: this might be redundant as asm might be saving this as well.
    pub asm_manifests: Vec<AsmManifest>,
}

// TODO: comprehensive tests
