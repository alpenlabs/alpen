use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
};

use reth_provider::{errors::db::DatabaseError, AccountReader, ProviderError, StateProvider};
use reth_revm::DatabaseRef;
use revm_primitives::{
    alloy_primitives::{ruint::Uint, Address, Bytes, B256, U256},
    AccountInfo, Bytecode,
};

/// `CacheDBProvider` implements a provider for the revm `CacheDB`.
/// In addition it holds accessed account info, storage values, and bytecodes during
/// transaction execution, supporting state retrieval for storage proof construction
/// in EL proof witness generation.
pub struct CacheDBProvider {
    provider: Box<dyn StateProvider>,
    accounts: RefCell<HashMap<Address, AccountInfo>>,
    storage: RefCell<HashMap<Address, HashMap<U256, U256>>>,
    bytecodes: RefCell<HashSet<Bytes>>,
    accessed_blkd_ids: RefCell<HashSet<u64>>,
}

#[derive(Debug)]
pub struct AccessedState {
    accessed_accounts: HashMap<Address, Vec<Uint<256, 4>>>,
    accessed_contracts: Vec<Bytes>,
    accessed_block_idxs: HashSet<u64>,
}

impl AccessedState {
    pub fn accessed_block_idxs(&self) -> &HashSet<u64> {
        &self.accessed_block_idxs
    }

    pub fn accessed_accounts(&self) -> &HashMap<Address, Vec<Uint<256, 4>>> {
        &self.accessed_accounts
    }

    pub fn accessed_contracts(&self) -> &Vec<Bytes> {
        &self.accessed_contracts
    }
}

impl CacheDBProvider {
    pub fn new(provider: Box<dyn StateProvider>) -> Self {
        Self {
            provider,
            accounts: Default::default(),
            storage: Default::default(),
            bytecodes: Default::default(),
            accessed_blkd_ids: Default::default(),
        }
    }

    pub fn get_accessed_state(&self) -> AccessedState {
        let accessed_accounts = self.get_accessed_accounts();
        let accessed_contracts = self.get_accessed_contracts();

        AccessedState {
            accessed_accounts,
            accessed_contracts,
            accessed_block_idxs: self.accessed_blkd_ids.borrow().clone(),
        }
    }

    fn get_accessed_accounts(&self) -> HashMap<Address, Vec<U256>> {
        let accounts = self.accounts.borrow();
        let storage = self.storage.borrow();

        accounts
            .keys()
            .chain(storage.keys())
            .copied()
            .collect::<HashSet<_>>()
            .into_iter()
            .map(|address| {
                let storage_keys = storage
                    .get(&address)
                    .map_or(Vec::new(), |map| map.keys().cloned().collect());
                (address, storage_keys)
            })
            .collect()
    }

    fn get_accessed_contracts(&self) -> Vec<Bytes> {
        self.bytecodes.borrow().iter().cloned().collect()
    }
}

impl DatabaseRef for CacheDBProvider {
    /// The database error type.
    type Error = ProviderError;

    /// Get basic account information.
    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let account_info = self
            .provider
            .basic_account(&address)?
            .map(|account| account.into());

        // Record the account value to the state.
        self.accounts
            .borrow_mut()
            .insert(address, account_info.clone().unwrap_or_default());

        Ok(account_info)
    }

    /// Get account code by its hash.
    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        let bytecode = self
            .provider
            .bytecode_by_hash(&code_hash)?
            .map(|code| Bytecode::new_raw(code.original_bytes()))
            .ok_or_else(|| {
                ProviderError::Database(DatabaseError::Other(format!(
                    "Bytecode for the given {code_hash:?} not found",
                )))
            })?;

        // Record the storage value to the state
        self.bytecodes
            .borrow_mut()
            .insert(bytecode.original_bytes().clone());

        Ok(bytecode)
    }

    /// Get storage value of address at index.
    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        let storage_value = self
            .provider
            .storage(address, index.into())?
            .unwrap_or(U256::default());

        // Record the storage value to the state.
        self.storage
            .borrow_mut()
            .entry(address)
            .or_default()
            .insert(index, storage_value);

        Ok(storage_value)
    }

    /// Get block hash by block number.
    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        let blk_id = self
            .provider
            .block_hash(number)?
            .ok_or(ProviderError::BlockBodyIndicesNotFound(number))?;

        self.accessed_blkd_ids.borrow_mut().insert(number);

        Ok(blk_id)
    }
}
