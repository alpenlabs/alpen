use std::collections::HashMap;

use alpen_chainspec::chain_value_parser;
use revm_primitives::{alloy_primitives::Address, B256, U256};
use strata_mpt::{keccak, MptNode, StateAccount, EMPTY_ROOT, KECCAK_EMPTY};

use crate::da::{DaAccountChange, DaEeStateDiff};
use crate::BatchStateDiff;

/// An (in-memory) representation of the EVM state reconstructed only from [`BatchStateDiff`].
#[derive(Clone, Default, Debug)]
pub struct ReconstructedState {
    state_trie: MptNode,
    storage_trie: HashMap<Address, MptNode>,
}

use thiserror::Error as ThisError;

/// A concrete error that may happen during state reconstruction.
#[derive(Debug, ThisError)]
pub enum StateError {
    #[error("Mpt Error: {0}")]
    MptError(#[from] strata_mpt::Error),
}

impl ReconstructedState {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn new_from_spec(spec: &str) -> Result<Self, eyre::Error> {
        let chain_spec = chain_value_parser(spec)?;

        let mut new = Self::new();
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
                    let acc_storage_trie = new.storage_trie.entry(*address).or_default();
                    for (slot_key, slot_value) in slots.iter() {
                        if slot_value != &B256::ZERO {
                            acc_storage_trie.insert_rlp(&keccak(slot_key), *slot_value)?;
                        }
                    }
                    state_account.storage_root = acc_storage_trie.hash();
                }
            }

            new.state_trie.insert_rlp(&keccak(address), state_account)?;
        }

        Ok(new)
    }

    /// Applies a single [`BatchStateDiff`] atop of the current State.
    pub fn apply(&mut self, batch_diff: BatchStateDiff) -> Result<(), StateError> {
        // Adjust accounts based on the accounts into and its slots.
        for (address, account_info) in batch_diff.accounts {
            let acc_info_trie_path = keccak(address);

            if let Some(account_info) = account_info {
                // Create a rlp-serializable StateAccount from the info.
                // Storage root is yet to be calculated below.
                let mut state_account = StateAccount {
                    nonce: account_info.nonce,
                    balance: account_info.balance,
                    storage_root: Default::default(),
                    code_hash: account_info.code_hash,
                };

                // Do not put empty accounts into the state trie.
                if state_account.is_account_empty() {
                    continue;
                }

                // Calculate the actual account storage root - take an existing storage trie
                // for the account and apply changed slots atop.
                state_account.storage_root = {
                    let acc_storage_trie = self.storage_trie.entry(address).or_default();
                    let changed_storage_slots = batch_diff.storage_slots.get(&address);

                    if let Some(changed_slots) = changed_storage_slots {
                        for (slot_key, slot_value) in changed_slots {
                            let slot_trie_path = keccak(slot_key.to_be_bytes::<32>());

                            if slot_value == &U256::ZERO {
                                acc_storage_trie.delete(&slot_trie_path)?;
                            } else {
                                acc_storage_trie.insert_rlp(&slot_trie_path, *slot_value)?;
                            }
                        }
                    }
                    acc_storage_trie.hash()
                };

                // Insert the up-to-date account into the trie.
                self.state_trie
                    .insert_rlp(&acc_info_trie_path, state_account)?;
            } else {
                // Account was actually destructed.
                self.state_trie.delete(&acc_info_trie_path)?;
            }
        }

        Ok(())
    }

    /// Applies a [`DaEeStateDiff`] atop of the current State.
    pub fn apply_da(&mut self, da_diff: &DaEeStateDiff) -> Result<(), StateError> {
        for (address, change) in &da_diff.accounts {
            let acc_info_trie_path = keccak(address);

            match change {
                DaAccountChange::Created(diff) | DaAccountChange::Updated(diff) => {
                    // Get current account state (if exists)
                    let current: Option<StateAccount> = self
                        .state_trie
                        .get_rlp(&acc_info_trie_path)
                        .unwrap_or_default();

                    let (current_balance, current_nonce, current_code_hash) = current
                        .map(|acc| (acc.balance, acc.nonce, acc.code_hash))
                        .unwrap_or((U256::ZERO, 0, KECCAK_EMPTY));

                    // Apply diff
                    let new_balance = diff
                        .balance
                        .new_value()
                        .map(|v| v.0)
                        .unwrap_or(current_balance);
                    let new_nonce = current_nonce + diff.nonce_incr.unwrap_or(0) as u64;
                    let new_code_hash = diff
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
                        if let Some(storage_diff) = da_diff.storage.get(address) {
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
                DaAccountChange::Deleted => {
                    self.state_trie.delete(&acc_info_trie_path)?;
                    self.storage_trie.remove(address);
                }
            }
        }

        // Handle storage changes for accounts not in accounts map
        // (e.g., storage-only changes)
        for (address, storage_diff) in &da_diff.storage {
            if da_diff.accounts.contains_key(address) {
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

    /// Get the current state root.
    pub fn state_root(&self) -> B256 {
        self.state_trie.hash()
    }

    /// Get the current storage root for a given account address.
    pub fn storage_root(&self, account_address: Address) -> B256 {
        self.storage_trie
            .get(&account_address)
            .map(|t| t.hash())
            .unwrap_or(EMPTY_ROOT)
    }

    /// Get the value for the given address at a given slot.
    pub fn storage_slot(&self, address: Address, slot_key: U256) -> U256 {
        self.storage_trie
            .get(&address)
            .unwrap_or(&MptNode::default())
            .get_rlp::<U256>(&keccak(slot_key.to_be_bytes::<32>()))
            .unwrap_or_default()
            .unwrap_or_default()
    }

    /// Get the account by address.
    pub fn account(&self, address: Address) -> Option<StateAccount> {
        self.state_trie
            .get_rlp(&keccak(address))
            .unwrap_or_default()
    }
}
