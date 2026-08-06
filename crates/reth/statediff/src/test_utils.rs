//! Shared test helpers for state-diff fixture construction and canonical-state oracles.
//!
//! This module centralizes deterministic builders used across `alpen-reth-statediff`
//! tests, including block-diff assembly helpers and canonical MPT-derived state
//! computations for reconstruction oracle checks.

use std::collections::BTreeMap;

use alloy_primitives::Bytes;
use revm_primitives::{Address, B256, U256};
use strata_mpt::{keccak, MptNode, StateAccount, EMPTY_ROOT};

use crate::{
    batch::{BatchBuilder, BatchStateDiff},
    block::{AccountSnapshot, BlockAccountChange, BlockStateChanges},
};

/// Canonical per-account storage view used by test oracles.
pub type AccountStorage = BTreeMap<U256, U256>;

/// Returns a deterministic address derived from a single-byte seed.
pub fn addr(seed: u8) -> Address {
    Address::from([seed; 20])
}

/// Returns a deterministic `B256` hash derived from a single-byte seed.
pub fn hash(seed: u8) -> B256 {
    B256::from([seed; 32])
}

/// Returns a storage-slot key from a small integer.
pub fn slot(value: u64) -> U256 {
    U256::from(value)
}

/// Returns a storage or balance value from a small integer.
pub fn value(value: u64) -> U256 {
    U256::from(value)
}

/// Copies bytecode into owned bytes for block-diff fixtures.
pub fn bytecode(bytes: &[u8]) -> Bytes {
    Bytes::copy_from_slice(bytes)
}

/// Builds a compact account snapshot for test fixtures.
pub fn snapshot(balance: u64, nonce: u64, code_hash: B256) -> AccountSnapshot {
    AccountSnapshot {
        balance: U256::from(balance),
        nonce,
        code_hash,
    }
}

/// Inserts an account-level change into a block diff fixture.
pub fn account_change(
    diff: &mut BlockStateChanges,
    address: Address,
    original: Option<AccountSnapshot>,
    current: Option<AccountSnapshot>,
) {
    diff.accounts
        .insert(address, BlockAccountChange { original, current });
}

/// Inserts a storage-slot change into a block diff fixture.
pub fn storage_change(
    diff: &mut BlockStateChanges,
    address: Address,
    slot_key: U256,
    original: U256,
    current: U256,
) {
    diff.storage
        .entry(address)
        .or_default()
        .slots
        .insert(slot_key, (original, current));
}

/// Records deployed bytecode in a block diff fixture.
pub fn deployed_bytecode(diff: &mut BlockStateChanges, code_hash: B256, deployed_bytecode: Bytes) {
    diff.deployed_bytecodes.insert(code_hash, deployed_bytecode);
}

/// Creates an empty per-block state diff fixture.
pub fn block_diff() -> BlockStateChanges {
    BlockStateChanges::new()
}

/// Aggregates a sequence of block diffs into a single batch diff.
pub fn batch_diff(blocks: &[BlockStateChanges]) -> BatchStateDiff {
    let mut builder = BatchBuilder::new();
    for block in blocks {
        builder.apply_block(block);
    }
    builder.build()
}

/// Canonical account and storage state used by reconstruction oracles.
#[derive(Clone, Debug, Default)]
pub struct CanonicalState {
    /// Final account records keyed by address.
    pub accounts: BTreeMap<Address, StateAccount>,
    /// Final storage contents keyed by address and slot.
    pub storage: BTreeMap<Address, AccountStorage>,
}

impl CanonicalState {
    /// Creates an empty canonical state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces a canonical account entry.
    pub fn with_account(mut self, address: Address, account: StateAccount) -> Self {
        self.accounts.insert(address, account);
        self
    }

    /// Sets a canonical storage slot value for an account.
    pub fn set_storage_slot(mut self, address: Address, slot_key: U256, slot_value: U256) -> Self {
        self.storage
            .entry(address)
            .or_default()
            .insert(slot_key, slot_value);
        self
    }

    /// Removes a canonical storage slot and prunes empty account storage maps.
    pub fn remove_storage_slot(mut self, address: Address, slot_key: U256) -> Self {
        if let Some(account_storage) = self.storage.get_mut(&address) {
            account_storage.remove(&slot_key);
            if account_storage.is_empty() {
                self.storage.remove(&address);
            }
        }
        self
    }
}

/// Builds a canonical `StateAccount` with an empty storage root placeholder.
pub fn state_account(balance: u64, nonce: u64, code_hash: B256) -> StateAccount {
    StateAccount {
        nonce,
        balance: U256::from(balance),
        storage_root: EMPTY_ROOT,
        code_hash,
    }
}

/// Builds canonical storage tries for every account present in the state view.
pub fn canonical_storage_tries(
    state: &CanonicalState,
) -> Result<BTreeMap<Address, MptNode>, strata_mpt::Error> {
    let mut storage_tries = BTreeMap::new();

    for (address, storage) in &state.storage {
        let mut storage_trie = MptNode::default();
        for (slot_key, slot_value) in storage {
            if slot_value.is_zero() {
                continue;
            }

            let slot_trie_path = keccak(slot_key.to_be_bytes::<32>());
            storage_trie.insert_rlp(&slot_trie_path, *slot_value)?;
        }
        storage_tries.insert(*address, storage_trie);
    }

    Ok(storage_tries)
}

/// Recomputes canonical accounts with storage roots derived from canonical storage.
pub fn canonical_accounts(
    state: &CanonicalState,
) -> Result<BTreeMap<Address, StateAccount>, strata_mpt::Error> {
    let storage_tries = canonical_storage_tries(state)?;
    let mut accounts = BTreeMap::new();

    for (address, account) in &state.accounts {
        let mut account = account.clone();
        account.storage_root = storage_tries
            .get(address)
            .map(MptNode::hash)
            .unwrap_or(EMPTY_ROOT);
        accounts.insert(*address, account);
    }

    Ok(accounts)
}

/// Computes the canonical global state root for the provided state view.
pub fn canonical_state_root(state: &CanonicalState) -> Result<B256, strata_mpt::Error> {
    let accounts = canonical_accounts(state)?;
    let mut state_trie = MptNode::default();

    for (address, account) in accounts {
        if account.is_account_empty() {
            continue;
        }
        state_trie.insert_rlp(&keccak(address), account)?;
    }

    Ok(state_trie.hash())
}
