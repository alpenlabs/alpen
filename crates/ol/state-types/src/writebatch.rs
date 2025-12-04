use std::collections::HashMap;

use strata_acct_types::AccountId;
use strata_identifiers::Buf32;
use strata_ledger_types::AsmManifest;
use strata_snark_acct_types::MessageEntry;

use crate::{AccountState, EpochalState, GlobalState};

/// Copy-on-Write overlay for state modifications during block execution.
///
/// This structure acts as a sparse, in-memory layer on top of a base state
/// (which could be DB-backed). Reads check the overlay first, then fall through
/// to the base. Writes populate the overlay.
#[derive(Clone, Debug, Default)]
pub struct WriteBatch {
    /// Accounts modified during execution (CoW overlay).
    /// Presence in this map indicates the account was accessed mutably.
    pub(crate) modified_accounts: HashMap<AccountId, AccountState>,

    /// Global state override. None means use base state.
    pub(crate) global_state: Option<GlobalState>,

    /// Epochal state override. None means use base state.
    pub(crate) epochal_state: Option<EpochalState>,
}

impl WriteBatch {
    /// Create a new empty WriteBatch
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if an account exists in the overlay
    pub fn has_account(&self, id: &AccountId) -> bool {
        self.modified_accounts.contains_key(id)
    }

    /// Get a reference to a modified account, if it exists in the overlay
    pub fn get_account(&self, id: &AccountId) -> Option<&AccountState> {
        self.modified_accounts.get(id)
    }

    /// Get a mutable reference to a modified account, if it exists in the overlay
    pub fn get_account_mut(&mut self, id: &AccountId) -> Option<&mut AccountState> {
        self.modified_accounts.get_mut(id)
    }

    /// Insert or update an account in the overlay
    pub fn insert_account(&mut self, id: AccountId, state: AccountState) {
        self.modified_accounts.insert(id, state);
    }

    /// Get global state if overridden
    pub fn global_state(&self) -> Option<&GlobalState> {
        self.global_state.as_ref()
    }

    /// Get mutable global state, creating if needed
    pub fn global_state_mut_or_insert(&mut self, base: &GlobalState) -> &mut GlobalState {
        self.global_state.get_or_insert_with(|| base.clone())
    }

    /// Get epochal state if overridden
    pub fn epochal_state(&self) -> Option<&EpochalState> {
        self.epochal_state.as_ref()
    }

    /// Get mutable epochal state, creating if needed
    pub fn epochal_state_mut_or_insert(&mut self, base: &EpochalState) -> &mut EpochalState {
        self.epochal_state.get_or_insert_with(|| base.clone())
    }

    /// Get the number of modified accounts
    pub fn modified_accounts_count(&self) -> usize {
        self.modified_accounts.len()
    }
}

/// Auxiliary data for database persistence (not part of consensus state root).
///
/// This data is tracked separately because it's used for DB indexing and queries,
/// but doesn't affect the consensus state root computation.
#[derive(Clone, Debug, Default)]
pub struct ExecutionAuxiliaryData {
    /// Messages added to each account's inbox during this block.
    /// Stored separately for DB indexing and queries.
    pub account_message_additions: HashMap<AccountId, Vec<MessageEntry>>,

    /// ASM manifests processed during this block.
    /// Stored separately for DB indexing.
    pub asm_manifests: Vec<AsmManifest>,
}
