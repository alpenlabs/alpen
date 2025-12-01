use std::collections::HashMap;

use strata_acct_types::{AccountId, AccountSerial, BitcoinAmount, Mmr64};
use strata_identifiers::{Buf32, EpochCommitment};
use strata_ledger_types::AsmManifest;
use strata_snark_acct_types::MessageEntry;

use crate::AccountState;

/// Batch of state changes from executing a block
#[derive(Clone, Debug)]
pub struct WriteBatch {
    // Global state changes
    pub new_slot: u64,

    // L1 View / Epochal state changes
    pub l1_view_writes: L1ViewWrites,

    // Account changes - store full AccountState for now
    // TODO: Consider storing diffs instead of full state for efficiency
    pub new_accounts: Vec<(AccountId, AccountSerial, AccountState)>,
    pub modified_accounts: HashMap<AccountId, AccountState>,

    // Final state root after all changes
    pub ledger_state_root: Buf32,
}

#[derive(Clone, Debug)]
pub struct L1ViewWrites {
    pub cur_epoch: u32,
    pub added_manifests: Vec<AsmManifest>,
    pub asm_manifest_mmr: Mmr64,
    pub asm_recorded_epoch: EpochCommitment,
    pub total_ledger_balance: BitcoinAmount,
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
