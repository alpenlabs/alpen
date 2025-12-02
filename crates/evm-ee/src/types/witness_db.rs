//! Witness database implementation for EVM execution.
//!
//! This module provides a custom DatabaseRef implementation that avoids unnecessary
//! clones by holding only references to pre-computed data in EvmPartialState.

use std::{cell::RefCell, collections::BTreeMap};

use reth_errors::ProviderError;
use reth_trie::TrieAccount;
use revm::{
    DatabaseRef,
    primitives::{Address, B256, U256, keccak256},
    state::{AccountInfo, Bytecode},
};
use revm_primitives::map::HashMap;
use rsp_mpt::EthereumState;

/// Custom witness database that implements DatabaseRef using only references.
///
/// Uses lazy hashing with caching: addresses and storage keys are hashed once on
/// first access, then cached for reuse. This avoids redundant hashing across
/// multiple accesses within a block and across blocks in batch execution.
pub struct WitnessDB<'a> {
    /// Reference to the sparse Merkle Patricia Trie state
    ethereum_state: &'a EthereumState,
    /// Reference to pre-computed block hashes map (no allocation needed)
    block_hashes: &'a HashMap<u64, B256>,
    /// Reference to bytecode map (no intermediate HashMap creation)
    bytecodes: &'a BTreeMap<B256, Bytecode>,
    /// Cache for hashed addresses (lazy computed, then reused)
    address_hash_cache: RefCell<HashMap<Address, B256>>,
    /// Cache for hashed storage keys (lazy computed, then reused)
    storage_key_cache: RefCell<HashMap<U256, B256>>,
}

impl<'a> std::fmt::Debug for WitnessDB<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WitnessDB")
            .field("ethereum_state", &self.ethereum_state)
            .field("block_hashes", &self.block_hashes)
            .field("bytecodes_count", &self.bytecodes.len())
            .field(
                "address_cache_size",
                &self.address_hash_cache.borrow().len(),
            )
            .field(
                "storage_key_cache_size",
                &self.storage_key_cache.borrow().len(),
            )
            .finish()
    }
}

impl<'a> WitnessDB<'a> {
    /// Creates a new WitnessDB with references to pre-computed data.
    ///
    /// Hash caches are initialized empty and populated lazily on first access.
    pub fn new(
        ethereum_state: &'a EthereumState,
        block_hashes: &'a HashMap<u64, B256>,
        bytecodes: &'a BTreeMap<B256, Bytecode>,
    ) -> Self {
        Self {
            ethereum_state,
            block_hashes,
            bytecodes,
            address_hash_cache: RefCell::new(HashMap::with_hasher(Default::default())),
            storage_key_cache: RefCell::new(HashMap::with_hasher(Default::default())),
        }
    }
}

impl<'a> DatabaseRef for WitnessDB<'a> {
    type Error = ProviderError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        // Check if address hash is cached, otherwise compute and cache it
        let hashed_address = {
            let cache = self.address_hash_cache.borrow();
            if let Some(&cached_hash) = cache.get(&address) {
                // Reuse cached hash - avoids redundant keccak256 computation
                cached_hash
            } else {
                // Not cached - compute hash once and store for future accesses
                drop(cache); // Release borrow before mutable borrow
                let hash = keccak256(address);
                self.address_hash_cache.borrow_mut().insert(address, hash);
                hash
            }
        };

        // Query the state trie for the account using cached/new hash
        let account_in_trie = self
            .ethereum_state
            .state_trie
            .get_rlp::<TrieAccount>(hashed_address.as_slice())
            .unwrap();

        // Convert TrieAccount to AccountInfo
        let account = account_in_trie.map(|account_in_trie| AccountInfo {
            balance: account_in_trie.balance,
            nonce: account_in_trie.nonce,
            code_hash: account_in_trie.code_hash,
            code: None,
        });

        Ok(account)
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        // Look up bytecode by hash and clone it (required by DatabaseRef trait)
        // This clone is unavoidable as the trait requires returning owned Bytecode,
        // but it happens only once per unique bytecode during execution (EVM caches it)
        Ok(self
            .bytecodes
            .get(&code_hash)
            .cloned()
            .expect("Bytecode must be present in witness"))
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        // Check if address hash is cached, otherwise compute and cache it
        let hashed_address = {
            let cache = self.address_hash_cache.borrow();
            if let Some(&cached_hash) = cache.get(&address) {
                // Reuse cached hash
                cached_hash
            } else {
                // Not cached - compute hash once and store
                drop(cache);
                let hash = keccak256(address);
                self.address_hash_cache.borrow_mut().insert(address, hash);
                hash
            }
        };

        // Get the storage trie for this account
        let storage_trie = self
            .ethereum_state
            .storage_tries
            .get(hashed_address.as_slice())
            .expect("A storage trie must be provided for each account");

        // Check if storage key hash is cached, otherwise compute and cache it
        let hashed_index = {
            let cache = self.storage_key_cache.borrow();
            if let Some(&cached_hash) = cache.get(&index) {
                // Reuse cached hash
                cached_hash
            } else {
                // Not cached - compute hash once and store
                drop(cache);
                let hash = keccak256(index.to_be_bytes::<32>());
                self.storage_key_cache.borrow_mut().insert(index, hash);
                hash
            }
        };

        // Query the storage trie using cached/new hashes
        Ok(storage_trie
            .get_rlp::<U256>(hashed_index.as_slice())
            .expect("Can get from MPT")
            .unwrap_or_default())
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        // Look up block hash by number - return a copy (B256 is Copy)
        Ok(self.block_hashes.get(&number).copied().unwrap_or_default())
    }
}
