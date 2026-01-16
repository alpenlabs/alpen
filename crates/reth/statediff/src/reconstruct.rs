//! State reconstruction from batch diffs.

use std::collections::HashMap;

use alpen_chainspec::chain_value_parser;
use revm_primitives::{alloy_primitives::Address, B256, U256};
use strata_mpt::{keccak, MptNode, StateAccount, EMPTY_ROOT, KECCAK_EMPTY};
use thiserror::Error as ThisError;

use crate::batch::{AccountChange, BatchStateDiff};

/// Error that may occur during state reconstruction.
#[derive(Debug, ThisError)]
pub enum ReconstructError {
    #[error("MPT error: {0}")]
    Mpt(#[from] strata_mpt::Error),
}

/// Reconstructs EVM state by applying [`BatchStateDiff`]s sequentially.
///
/// Used primarily for testing to verify that state roots reconstructed
/// from diffs match the actual state roots from EE blocks.
#[derive(Clone, Default, Debug)]
pub struct StateReconstructor {
    state_trie: MptNode,
    storage_trie: HashMap<Address, MptNode>,
}

impl StateReconstructor {
    /// Creates a new empty reconstructor.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a reconstructor initialized with genesis state from a chain spec.
    pub fn from_chain_spec(spec: &str) -> Result<Self, eyre::Error> {
        let chain_spec = chain_value_parser(spec)?;

        let mut reconstructor = Self::new();
        for (address, account) in chain_spec.genesis.alloc.iter() {
            let mut state_account = StateAccount {
                nonce: account.nonce.unwrap_or(0),
                balance: account.balance,
                storage_root: EMPTY_ROOT,
                code_hash: account
                    .code
                    .as_ref()
                    .map(|bytes| keccak(bytes).into())
                    .unwrap_or(KECCAK_EMPTY),
            };

            if let Some(slots) = &account.storage {
                if !slots.is_empty() {
                    let acc_storage_trie = reconstructor.storage_trie.entry(*address).or_default();
                    for (slot_key, slot_value) in slots.iter() {
                        if slot_value != &B256::ZERO {
                            acc_storage_trie.insert_rlp(&keccak(slot_key), *slot_value)?;
                        }
                    }
                    state_account.storage_root = acc_storage_trie.hash();
                }
            }

            reconstructor
                .state_trie
                .insert_rlp(&keccak(address), state_account)?;
        }

        Ok(reconstructor)
    }

    /// Applies a [`BatchStateDiff`] to the current state.
    pub fn apply_diff(&mut self, diff: &BatchStateDiff) -> Result<(), ReconstructError> {
        for (address, change) in &diff.accounts {
            let acc_info_trie_path = keccak(address);

            match change {
                AccountChange::Created(account_diff) | AccountChange::Updated(account_diff) => {
                    // Get current account state (if exists)
                    let current: Option<StateAccount> = self
                        .state_trie
                        .get_rlp(&acc_info_trie_path)
                        .unwrap_or_default();

                    let (current_balance, current_nonce, current_code_hash) = current
                        .map(|acc| (acc.balance, acc.nonce, acc.code_hash))
                        .unwrap_or((U256::ZERO, 0, KECCAK_EMPTY));

                    // Apply diff
                    let new_balance = account_diff
                        .balance
                        .new_value()
                        .map(|v| v.0)
                        .unwrap_or(current_balance);
                    let nonce_delta = account_diff
                        .nonce
                        .diff()
                        .map(|v| v.inner() as u64)
                        .unwrap_or(0);
                    let new_nonce = current_nonce + nonce_delta;
                    let new_code_hash = account_diff
                        .code_hash
                        .new_value()
                        .map(|v| v.0)
                        .unwrap_or(current_code_hash);

                    let mut state_account = StateAccount {
                        nonce: new_nonce,
                        balance: new_balance,
                        storage_root: Default::default(),
                        code_hash: new_code_hash,
                    };

                    // Skip empty accounts
                    if state_account.is_account_empty() {
                        continue;
                    }

                    // Calculate storage root
                    state_account.storage_root = {
                        let acc_storage_trie = self.storage_trie.entry(*address).or_default();
                        if let Some(storage_diff) = diff.storage.get(address) {
                            for (slot_key, slot_value) in storage_diff.iter() {
                                let slot_trie_path = keccak(slot_key.to_be_bytes::<32>());
                                match slot_value {
                                    Some(v) if !v.is_zero() => {
                                        acc_storage_trie.insert_rlp(&slot_trie_path, *v)?;
                                    }
                                    _ => {
                                        acc_storage_trie.delete(&slot_trie_path)?;
                                    }
                                }
                            }
                        }
                        acc_storage_trie.hash()
                    };

                    self.state_trie
                        .insert_rlp(&acc_info_trie_path, state_account)?;
                }
                AccountChange::Deleted => {
                    self.state_trie.delete(&acc_info_trie_path)?;
                    self.storage_trie.remove(address);
                }
            }
        }

        // Handle storage changes for accounts not in accounts map
        // (e.g., storage-only changes)
        for (address, storage_diff) in &diff.storage {
            if diff.accounts.contains_key(address) {
                continue; // Already handled above
            }

            let acc_info_trie_path = keccak(address);
            let current: Option<StateAccount> = self
                .state_trie
                .get_rlp(&acc_info_trie_path)
                .unwrap_or_default();

            if let Some(mut state_account) = current {
                let acc_storage_trie = self.storage_trie.entry(*address).or_default();
                for (slot_key, slot_value) in storage_diff.iter() {
                    let slot_trie_path = keccak(slot_key.to_be_bytes::<32>());
                    match slot_value {
                        Some(v) if !v.is_zero() => {
                            acc_storage_trie.insert_rlp(&slot_trie_path, *v)?;
                        }
                        _ => {
                            acc_storage_trie.delete(&slot_trie_path)?;
                        }
                    }
                }
                state_account.storage_root = acc_storage_trie.hash();
                self.state_trie
                    .insert_rlp(&acc_info_trie_path, state_account)?;
            }
        }

        Ok(())
    }

    /// Returns the current state root.
    pub fn state_root(&self) -> B256 {
        self.state_trie.hash()
    }

    /// Returns the current storage root for an account.
    pub fn storage_root(&self, address: Address) -> B256 {
        self.storage_trie
            .get(&address)
            .map(|t| t.hash())
            .unwrap_or(EMPTY_ROOT)
    }

    /// Returns the value at a storage slot.
    pub fn storage_slot(&self, address: Address, slot_key: U256) -> U256 {
        self.storage_trie
            .get(&address)
            .unwrap_or(&MptNode::default())
            .get_rlp::<U256>(&keccak(slot_key.to_be_bytes::<32>()))
            .unwrap_or_default()
            .unwrap_or_default()
    }

    /// Returns the account state.
    pub fn account(&self, address: Address) -> Option<StateAccount> {
        self.state_trie
            .get_rlp(&keccak(address))
            .unwrap_or_default()
    }
}
