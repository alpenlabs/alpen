// This code is modified from the original implementation of Zeth.
//
// Reference: https://github.com/risc0/zeth
//
// Copyright 2023 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either strata or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::collections::hash_map::Entry;

// use hashbrown::hash_map::Entry;
use alloy_primitives::map::{DefaultHashBuilder, HashMap};
use anyhow::{anyhow, Result};
use revm::{
    bytecode::Bytecode,
    database::{AccountState, Cache, DbAccount, InMemoryDB},
    state::AccountInfo,
};
use revm_primitives::alloy_primitives::{Address, Bytes, B256, U256};

use crate::{
    mpt::{keccak, StateAccount, KECCAK_EMPTY},
    EvmBlockStfInput,
};

/// A helper trait to extend [`InMemoryDB`] with additional functionality.
pub trait InMemoryDBHelper {
    /// Create an [`InMemoryDB`] from a given [`EvmBlockStfInput`].
    fn initialize(input: &mut EvmBlockStfInput) -> Result<Self>
    where
        Self: Sized;

    /// Get the account info for a given address.
    fn get_account_info(&self, address: Address) -> Result<Option<AccountInfo>>;

    /// Get the storage value of an address at an index.
    fn get_storage_slot(&self, address: Address, index: U256) -> Result<U256>;

    /// Get the storage keys for all accounts in the database.
    fn storage_keys(&self) -> HashMap<Address, Vec<U256>>;

    /// Insert block hash into the database.
    fn insert_block_hash(&mut self, block_number: U256, block_hash: B256);
}

impl InMemoryDBHelper for InMemoryDB {
    fn initialize(input: &mut EvmBlockStfInput) -> Result<Self> {
        // For each contract's byte code, hash it and store it in a map.
        let contracts: HashMap<B256, Bytes> = input
            .contracts
            .iter()
            .map(|bytes| (keccak(bytes).into(), bytes.clone()))
            .collect();

        // For each account, load the information into the database.
        let mut accounts = HashMap::with_capacity_and_hasher(
            input.pre_state_storage.len(),
            DefaultHashBuilder::default(),
        );
        for (address, (storage_trie, slots)) in &mut input.pre_state_storage {
            let state_account = input
                .pre_state_trie
                .get_rlp::<StateAccount>(&keccak(address))?
                .unwrap_or_default();

            if storage_trie.hash() != state_account.storage_root {
                panic!(
                    "Storage trie root does not match for account {:?}: expected {}, got {}",
                    address,
                    state_account.storage_root,
                    storage_trie.hash()
                );
            }

            let bytecode = if state_account.code_hash.0 == KECCAK_EMPTY.0 {
                Bytecode::default()
            } else {
                // N.B. It can happen that contract's code isn't present in the witness,
                // but it's *only* possible (as a special case) for SELFDESTRUCT opcode.
                // For such a case the code is not required because the contract's
                // balance is modified directly (without fallback or receive methods).
                // So it's ok to fallback to default code in such a case.
                let bytes = contracts
                    .get(&state_account.code_hash)
                    .unwrap_or_default()
                    .clone();
                Bytecode::new_raw(bytes)
            };

            let mut storage =
                HashMap::with_capacity_and_hasher(slots.len(), DefaultHashBuilder::default());
            for slot in slots {
                let value: U256 = storage_trie
                    .get_rlp(&keccak(slot.to_be_bytes::<32>()))?
                    .unwrap_or_default();
                storage.insert(*slot, value);
            }

            let account = DbAccount {
                info: AccountInfo {
                    balance: state_account.balance,
                    nonce: state_account.nonce,
                    code_hash: state_account.code_hash,
                    code: Some(bytecode),
                },
                account_state: AccountState::None,
                storage,
            };
            accounts.insert(*address, account);
        }

        // Insert ancestor headers into the database.
        let mut block_hashes = HashMap::with_capacity_and_hasher(
            input.ancestor_headers.len() + 1,
            DefaultHashBuilder::default(),
        );
        block_hashes.insert(
            U256::from(input.parent_header.number),
            input.parent_header.hash_slow(),
        );
        let mut prev = &input.parent_header;
        for current in &input.ancestor_headers {
            let current_hash = current.hash_slow();
            if prev.parent_hash != current_hash {
                panic!(
                    "Invalid chain: {} is not the parent of {}",
                    current.number, prev.number
                );
            }
            if input.parent_header.number < current.number
                || input.parent_header.number - current.number >= 256
            {
                panic!(
                    "Invalid chain: {} is not one of the {} most recent blocks",
                    current.number, 256,
                );
            }
            block_hashes.insert(U256::from(current.number), current_hash);
            prev = current;
        }

        // Return the DB.
        Ok(InMemoryDB {
            cache: Cache {
                accounts: accounts.clone(),
                block_hashes: block_hashes.clone(),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    fn get_account_info(&self, address: Address) -> Result<Option<AccountInfo>> {
        match self.cache.accounts.get(&address) {
            Some(db_account) => Ok(db_account.info()),
            None => Err(anyhow!("Account not found.")),
        }
    }

    fn get_storage_slot(&self, address: Address, index: U256) -> Result<U256> {
        match self.cache.accounts.get(&address) {
            Some(account) => match account.storage.get(&index) {
                Some(value) => Ok(*value),
                None => match account.account_state {
                    AccountState::NotExisting => unreachable!(),
                    AccountState::StorageCleared => Ok(U256::ZERO),
                    _ => Err(anyhow!("Storage slot not found.")),
                },
            },
            None => Err(anyhow!("Account not found.")),
        }
    }

    fn storage_keys(&self) -> HashMap<Address, Vec<U256>> {
        let mut out = HashMap::with_hasher(alloy_primitives::map::DefaultHashBuilder::default());
        for (address, account) in &self.cache.accounts {
            out.insert(*address, account.storage.keys().cloned().collect());
        }
        out
    }

    fn insert_block_hash(&mut self, block_number: U256, block_hash: B256) {
        match self.cache.block_hashes.entry(block_number) {
            Entry::Occupied(entry) => assert_eq!(&block_hash, entry.get()),
            Entry::Vacant(entry) => {
                entry.insert(block_hash);
            }
        };
    }
}
