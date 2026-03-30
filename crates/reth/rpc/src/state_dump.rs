//! Implementation of `strataee_dumpState` RPC method.
//!
//! Dumps the complete EVM state at the latest block. Designed for L2 chains
//! with small state. Not suitable for large chains (mainnet).

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use alloy_consensus::BlockHeader;
use alloy_primitives::{Address, BlockNumber, Bytes, StorageKey, StorageValue, B256, U256};
use jsonrpsee::core::RpcResult;
use reth_db::{cursor::DbCursorRO, tables, transaction::DbTx};
use reth_provider::{AccountExtReader, BlockNumReader, DatabaseProviderFactory, HeaderProvider};
use reth_storage_api::DBProvider;
use serde::{Deserialize, Serialize};
use strata_rpc_utils::to_jsonrpsee_error;
use tracing::info;

/// State dump of all accounts at a given block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateDump {
    /// State root at the dumped block.
    pub root: B256,
    /// Block number at which the state was dumped.
    pub block_number: BlockNumber,
    /// All accounts keyed by address.
    pub accounts: BTreeMap<Address, AccountDump>,
}

/// Dump of a single account's state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountDump {
    pub balance: U256,
    pub nonce: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Bytes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage: Option<BTreeMap<StorageKey, StorageValue>>,
}

/// Dumps the complete state at the latest block.
///
/// Only supports the latest block — `PlainAccountState` and `PlainStorageState`
/// tables reflect the current tip. Historical dumps are not supported.
///
/// All data is read from a **single MDBX read transaction** to ensure a
/// consistent snapshot even if new blocks land concurrently.
///
/// This works by:
/// 1. Opening a single read-only DB transaction (snapshot isolation).
/// 2. Reading the latest block number and header from that transaction.
/// 3. Walking `PlainAccountState` to collect all accounts with their balance/nonce.
/// 4. Supplementing with `changed_accounts_with_range(0..=latest)` for genesis accounts.
/// 5. Reading bytecode from `Bytecodes` table for accounts with code.
/// 6. Reading storage from `PlainStorageState` for each account.
pub fn dump_state_at_latest<F>(provider_factory: &F) -> eyre::Result<StateDump>
where
    F: DatabaseProviderFactory,
    F::Provider: AccountExtReader + BlockNumReader + HeaderProvider,
{
    // Open a single read transaction — all reads below see the same snapshot.
    let db_provider = provider_factory.database_provider_ro()?;
    let block_number = db_provider.last_block_number()?;

    info!(block_number, "starting state dump");

    // Get block header for state root (from the same transaction)
    let header = db_provider
        .header_by_number(block_number)?
        .ok_or_else(|| eyre::eyre!("block header not found for block {block_number}"))?;
    let state_root = header.state_root();

    // 1. Collect all accounts from PlainAccountState (balance, nonce, bytecode_hash)
    let tx = db_provider.tx_ref();
    let mut cursor = tx.cursor_read::<tables::PlainAccountState>()?;

    let mut account_entries = BTreeMap::new();
    let mut entry = cursor.first()?;
    while let Some((address, account)) = entry {
        account_entries.insert(address, account);
        entry = cursor.next()?;
    }

    // 2. Supplement with changesets to catch genesis-era accounts that may exist only in changesets
    //    (e.g. accounts created at genesis but later self-destructed and removed from
    //    PlainAccountState)
    let changeset_addresses = db_provider.changed_accounts_with_range(0..=block_number)?;
    let mut addresses: BTreeSet<Address> = account_entries.keys().copied().collect();
    addresses.extend(changeset_addresses);

    info!(
        account_count = addresses.len(),
        "collected addresses, building dump"
    );

    // 3. Pre-read all storage in a single cursor walk, grouped by address
    let mut all_storage: BTreeMap<Address, BTreeMap<StorageKey, StorageValue>> = BTreeMap::new();
    let mut storage_cursor = tx.cursor_dup_read::<tables::PlainStorageState>()?;
    let mut storage_entry = storage_cursor.first()?;
    while let Some((address, se)) = storage_entry {
        all_storage
            .entry(address)
            .or_default()
            .insert(se.key, se.value);
        storage_entry = storage_cursor.next()?;
    }

    // Also register any address that has storage but wasn't in PlainAccountState
    for addr in all_storage.keys() {
        addresses.insert(*addr);
    }

    // 4. Build the account dump
    let mut accounts = BTreeMap::new();
    for address in &addresses {
        let account = account_entries.get(address);

        // Read bytecode if present
        let code = account
            .and_then(|a| a.bytecode_hash)
            .and_then(|code_hash| tx.get::<tables::Bytecodes>(code_hash).transpose())
            .transpose()?
            .map(|b| Bytes::from(b.bytes().to_vec()));

        let storage = all_storage.remove(address);

        let (balance, nonce) = match account {
            Some(a) => (a.balance, a.nonce),
            None => (U256::ZERO, 0),
        };

        // Skip truly empty accounts (no balance, no nonce, no code, no storage)
        if balance.is_zero() && nonce == 0 && code.is_none() && storage.is_none() {
            continue;
        }

        accounts.insert(
            *address,
            AccountDump {
                balance,
                nonce,
                code,
                storage,
            },
        );
    }

    info!(
        block_number,
        state_root = %state_root,
        accounts = accounts.len(),
        "state dump complete"
    );

    Ok(StateDump {
        root: state_root,
        block_number,
        accounts,
    })
}

/// RPC module that provides state dump functionality.
#[derive(Debug)]
pub struct StateDumpRpc<F> {
    provider_factory: Arc<F>,
}

impl<F> StateDumpRpc<F> {
    /// Create a new `StateDumpRpc` instance.
    pub fn new(provider_factory: Arc<F>) -> Self {
        Self { provider_factory }
    }
}

impl<F> crate::StateDumpRpcApiServer for StateDumpRpc<F>
where
    F: DatabaseProviderFactory + Send + Sync + 'static,
    F::Provider: AccountExtReader + BlockNumReader + HeaderProvider,
{
    fn dump_state(&self, _block_number: Option<BlockNumber>) -> RpcResult<StateDump> {
        // Always dumps the latest block. The block_number parameter is accepted
        // for forward compatibility but currently ignored.
        dump_state_at_latest(self.provider_factory.as_ref())
            .map_err(to_jsonrpsee_error("failed to dump state"))
    }
}
