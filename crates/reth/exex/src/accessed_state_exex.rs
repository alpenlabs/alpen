//! Per-block accessed-state capture exex.
//!
//! Runs in parallel with [`crate::StateDiffGenerator`] but for a different
//! consumer: writes the *read set* (accounts, slots, code hashes, ancestor
//! block heights for BLOCKHASH) of each committed block to the
//! `AccessedStateStore`. The chunk-builder at chunk-seal time reads these
//! records to build the chunk witness without re-executing blocks itself.
//!
//! Capture path: re-execute each committed block here, wrapped in a
//! [`CacheDBProvider`] that records every account/slot/bytecode read.
//! Reth has already executed the block once before the exex notification
//! fires; we pay that re-execution cost as the price of staying out of
//! reth's EVM customization layer. Production-time historical depth is 1
//! (`history_by_block_number(blk - 1)`), so memory cost is bounded
//! regardless of chain age.
//!
//! ### Reorg handling
//!
//! On `ChainReorged` / `ChainReverted` notifications, the exex deletes
//! the accessed-state records for the orphaned block hashes. Bytecodes
//! are content-addressed and never deleted — same contract referenced by
//! many chunks shares one stored copy.

use std::{collections::BTreeMap, sync::Arc};

use alloy_eips::BlockNumHash;
use alloy_primitives::{keccak256, B256, KECCAK256_EMPTY};
use alpen_ee_common::{AccessedAccount, AccessedStateRecord, AccessedStateStore};
use alpen_reth_witness::CacheDBProvider;
use futures_util::TryStreamExt;
use reth_evm::{
    execute::{BasicBlockExecutor, Executor},
    ConfigureEvm,
};
use reth_exex::{ExExContext, ExExEvent};
use reth_node_api::{FullNodeComponents, NodeTypes};
use reth_primitives::{Block, EthPrimitives};
use reth_primitives_traits::Block as _;
use reth_provider::{BlockReader, Chain, StateProviderFactory};
use reth_revm::{
    db::{BundleState, CacheDB, CacheState},
    state::Bytecode,
};
use strata_acct_types::Hash;
use tokio::task;
use tracing::{debug, error, warn};

#[expect(
    missing_debug_implementations,
    reason = "Provider / evm config / store inner types don't implement Debug"
)]
pub struct AccessedStateGenerator<
    Node: FullNodeComponents<Types: NodeTypes<Primitives = EthPrimitives>>,
    S: AccessedStateStore + 'static,
> {
    ctx: ExExContext<Node>,
    store: Arc<S>,
}

impl<
        Node: FullNodeComponents<Types: NodeTypes<Primitives = EthPrimitives>>,
        S: AccessedStateStore + 'static,
    > AccessedStateGenerator<Node, S>
where
    Node::Provider: StateProviderFactory + BlockReader<Block = Block> + Clone + Send + Sync,
    Node::Evm: ConfigureEvm<Primitives = EthPrimitives> + Clone + Send + Sync,
{
    pub fn new(ctx: ExExContext<Node>, store: Arc<S>) -> Self {
        Self { ctx, store }
    }

    pub async fn start(mut self) -> eyre::Result<()> {
        debug!("start accessed state generator");
        while let Some(notification) = self.ctx.notifications.try_next().await? {
            if let Some(reverted) = notification.reverted_chain() {
                if let Err(err) = self.revert(&reverted).await {
                    error!(?err, "failed to revert accessed-state records");
                }
            }
            if let Some(committed) = notification.committed_chain() {
                match self.commit(&committed).await {
                    Ok(Some(height)) => {
                        if let Err(err) = self.ctx.events.send(ExExEvent::FinishedHeight(height)) {
                            warn!(?err, "failed to send FinishedHeight");
                        }
                    }
                    Ok(None) => {}
                    Err(err) => {
                        error!(?err, "failed to commit accessed-state records");
                    }
                }
            }
        }
        Ok(())
    }

    /// Re-execute every block in `chain` with a `CacheDBProvider` and
    /// persist the resulting accessed-state record + any new bytecodes.
    ///
    /// Returns the latest `(number, hash)` successfully processed so the
    /// caller can emit `FinishedHeight`.
    async fn commit(&self, chain: &Chain) -> eyre::Result<Option<BlockNumHash>> {
        let mut finished = None;
        let blocks = chain.blocks();
        for block_number in chain.range() {
            let Some(block) = blocks.get(&block_number) else {
                continue;
            };
            let block_hash = block.hash();

            let provider = self.ctx.provider().clone();
            let evm_config = self.ctx.evm_config().clone();
            let block_num = block_number;

            // Heavy lifting (re-execution + state-provider traversal) runs
            // off the async runtime.
            let record_result =
                task::spawn_blocking(move || build_accessed_state(provider, evm_config, block_num))
                    .await
                    .map_err(|e| eyre::eyre!("accessed-state join: {e}"))?;

            let (record, bytecodes) = match record_result {
                Ok(v) => v,
                Err(err) => {
                    error!(
                        ?err,
                        ?block_hash,
                        block_num,
                        "accessed-state extraction failed; halting commit to keep \
                         FinishedHeight contiguous (reth will redeliver on next notification)"
                    );
                    break;
                }
            };

            // Persist bytecodes first (content-addressed, idempotent), then
            // the per-block record. A bytecode failure is fatal for this
            // block: the record we're about to write references the hash,
            // and downstream witness extraction errors out on a missing
            // bytecode lookup. Halt the commit so `finished` stays
            // contiguous — reth will redeliver the block on the next
            // notification and we'll retry the whole step.
            let mut bytecode_failed = false;
            for (code_hash, code) in bytecodes {
                if let Err(err) = self.store.put_bytecode(code_hash, code).await {
                    error!(
                        ?err,
                        ?code_hash,
                        ?block_hash,
                        block_num,
                        "failed to persist bytecode; halting commit"
                    );
                    bytecode_failed = true;
                    break;
                }
            }
            if bytecode_failed {
                break;
            }

            if let Err(err) = self
                .store
                .put_block_accessed_state(hash_from_b256(block_hash), record)
                .await
            {
                error!(
                    ?err,
                    ?block_hash,
                    block_num,
                    "failed to persist accessed-state record; halting commit to keep \
                     FinishedHeight contiguous"
                );
                break;
            }

            debug!(?block_hash, block_num, "persisted accessed-state record");
            finished = Some(BlockNumHash::new(block_num, block_hash));
        }
        Ok(finished)
    }

    /// Delete accessed-state records for every block in the orphaned chain.
    /// Bytecodes are left in place — they're content-addressed and harmless
    /// to retain.
    async fn revert(&self, chain: &Chain) -> eyre::Result<()> {
        for block_number in chain.range() {
            let Some(block) = chain.blocks().get(&block_number) else {
                continue;
            };
            let block_hash = block.hash();
            if let Err(err) = self
                .store
                .del_block_accessed_state(hash_from_b256(block_hash))
                .await
            {
                warn!(
                    ?err,
                    ?block_hash,
                    "failed to delete reorged accessed-state record"
                );
            }
        }
        Ok(())
    }
}

/// `(code_hash, raw_bytecode)` pair returned alongside each block's
/// accessed-state record so the caller can persist bytecodes into the
/// content-addressed bytecode tree.
type BytecodeEntry = (Hash, Vec<u8>);

/// CPU-heavy half of `commit`, hoisted out so it can run inside
/// [`tokio::task::spawn_blocking`]. Reads the parent state via reth
/// (depth = 1 at production time), re-executes the block, and extracts
/// the `(record, bytecodes)` pair.
fn build_accessed_state<P, E>(
    provider: P,
    evm_config: E,
    block_num: u64,
) -> eyre::Result<(AccessedStateRecord, Vec<BytecodeEntry>)>
where
    P: StateProviderFactory + BlockReader<Block = Block>,
    E: ConfigureEvm<Primitives = EthPrimitives> + Clone,
{
    let block = provider
        .block_by_number(block_num)?
        .ok_or_else(|| eyre::eyre!("block {} not found", block_num))?;

    let sealed = block.seal_slow();
    let recovered = sealed.try_recover()?;

    let history = provider.history_by_block_number(block_num.saturating_sub(1))?;
    let cache_provider = CacheDBProvider::new(history);
    let cache_db = CacheDB::new(&cache_provider);

    let mut executor = BasicBlockExecutor::new(evm_config, cache_db);
    executor.execute_one(&recovered)?;
    let execution_state = executor.into_state();

    let accessed = cache_provider.get_accessed_state();

    let mut accounts: Vec<AccessedAccount> = accessed
        .accessed_accounts()
        .iter()
        .map(|(addr, slots)| {
            let mut storage_slots: Vec<[u8; 32]> =
                slots.iter().map(|slot| slot.to_be_bytes::<32>()).collect();
            storage_slots.sort();
            AccessedAccount {
                address: addr.into_array(),
                storage_slots,
            }
        })
        .collect();
    accounts.sort_by_key(|a| a.address);

    let collected_bytecodes = collect_block_bytecodes(
        accessed.accessed_contracts(),
        &execution_state.cache,
        &execution_state.bundle_state,
    )?;
    let bytecode_hashes = collected_bytecodes.iter().map(|(hash, _)| hash.0).collect();

    let mut ancestor_block_numbers: Vec<u64> =
        accessed.accessed_block_idxs().iter().copied().collect();
    ancestor_block_numbers.sort();

    let record = AccessedStateRecord {
        accounts,
        bytecode_hashes,
        ancestor_block_numbers,
    };

    let bytecodes = collected_bytecodes
        .into_iter()
        .map(|(hash, code)| (hash_from_b256(hash), code))
        .collect();

    Ok((record, bytecodes))
}

/// Collects every bytecode that may be required to replay a block.
///
/// The access-tracking database records code fetched from the parent state, but
/// revm can satisfy code loads from its in-memory execution state without
/// consulting that database. This happens notably for code installed by an
/// earlier transaction and for EIP-7702 delegation designations. Both the live
/// execution cache and the merged execution bundle are therefore required
/// sources of witness bytecode.
fn collect_block_bytecodes<'a>(
    accessed_contracts: impl IntoIterator<Item = (&'a B256, &'a Bytecode)>,
    execution_cache: &CacheState,
    bundle: &BundleState,
) -> eyre::Result<Vec<(B256, Vec<u8>)>> {
    let mut bytecodes = BTreeMap::new();

    for (hash, code) in accessed_contracts {
        insert_bytecode(&mut bytecodes, *hash, code, "access tracker")?;
    }
    for (hash, code) in &execution_cache.contracts {
        insert_bytecode(&mut bytecodes, *hash, code, "execution cache")?;
    }
    for account in execution_cache.accounts.values() {
        let Some(info) = account.account.as_ref().map(|account| &account.info) else {
            continue;
        };
        let Some(code) = &info.code else {
            continue;
        };
        if info.is_empty_code_hash() {
            continue;
        }
        insert_bytecode(
            &mut bytecodes,
            info.code_hash,
            code,
            "execution cache account info",
        )?;
    }
    for (hash, code) in &bundle.contracts {
        insert_bytecode(&mut bytecodes, *hash, code, "execution bundle")?;
    }
    for account in bundle.state.values() {
        for info in [account.original_info.as_ref(), account.info.as_ref()]
            .into_iter()
            .flatten()
        {
            let Some(code) = &info.code else {
                continue;
            };
            if info.is_empty_code_hash() {
                continue;
            }
            insert_bytecode(
                &mut bytecodes,
                info.code_hash,
                code,
                "execution account info",
            )?;
        }
    }

    Ok(bytecodes.into_iter().collect())
}

fn insert_bytecode(
    bytecodes: &mut BTreeMap<B256, Vec<u8>>,
    expected_hash: B256,
    code: &Bytecode,
    source: &'static str,
) -> eyre::Result<()> {
    let bytes = code.original_bytes();
    if bytes.is_empty() && (expected_hash.is_zero() || expected_hash == KECCAK256_EMPTY) {
        return Ok(());
    }

    let actual_hash = keccak256(&bytes);
    if actual_hash != expected_hash {
        eyre::bail!(
            "{source} bytecode hash mismatch: expected={expected_hash}, actual={actual_hash}"
        );
    }

    if let Some(existing) = bytecodes.get(&expected_hash) {
        if existing.as_slice() != bytes.as_ref() {
            eyre::bail!("conflicting bytecodes for hash {expected_hash}: source={source}");
        }
        return Ok(());
    }

    bytecodes.insert(expected_hash, bytes.to_vec());
    Ok(())
}

fn hash_from_b256(hash: B256) -> Hash {
    Hash::from(hash.0)
}

#[cfg(test)]
mod tests {
    use std::iter::{empty, once};

    use alloy_primitives::{Address, Bytes, U256};
    use reth_revm::{
        db::{AccountStatus, BundleAccount},
        state::AccountInfo,
    };

    use super::*;

    #[test]
    fn bytecode_entries_preserve_original_runtime_bytes() {
        let runtime = Bytes::from_static(&[0x60, 0x01, 0x5f, 0x55]);
        let bytecode = Bytecode::new_raw(runtime.clone());
        let code_hash = bytecode.hash_slow();

        let entries = collect_block_bytecodes(
            once((&code_hash, &bytecode)),
            &CacheState::default(),
            &BundleState::default(),
        )
        .unwrap();

        assert_eq!(entries, vec![(code_hash, runtime.to_vec())]);
    }

    #[test]
    fn captures_execution_cache_contract_missing_from_other_sources() {
        let code = Bytecode::new_raw(Bytes::from_static(&[0x60, 0x2a, 0x60, 0x00, 0x52]));
        let code_hash = code.hash_slow();
        let mut execution_cache = CacheState::default();
        execution_cache.contracts.insert(code_hash, code.clone());

        let entries =
            collect_block_bytecodes(empty(), &execution_cache, &BundleState::default()).unwrap();

        assert_eq!(entries, vec![(code_hash, code.original_bytes().to_vec())]);
    }

    #[test]
    fn captures_execution_cache_account_attached_code() {
        let code = Bytecode::new_eip7702(Address::repeat_byte(0x33));
        let code_hash = code.hash_slow();
        let info = AccountInfo {
            balance: U256::ZERO,
            nonce: 1,
            code_hash,
            code: Some(code.clone()),
        };
        let mut execution_cache = CacheState::default();
        execution_cache.insert_account(Address::repeat_byte(0x77), info);

        let entries =
            collect_block_bytecodes(empty(), &execution_cache, &BundleState::default()).unwrap();

        assert_eq!(entries, vec![(code_hash, code.original_bytes().to_vec())]);
    }

    #[test]
    fn captures_account_attached_code_missing_from_contract_maps() {
        let address = Address::repeat_byte(0x42);
        let code = Bytecode::new_eip7702(Address::repeat_byte(0x24));
        let code_hash = code.hash_slow();
        let info = AccountInfo {
            balance: U256::ZERO,
            nonce: 1,
            code_hash,
            code: Some(code.clone()),
        };
        let mut bundle = BundleState::default();
        bundle.state.insert(
            address,
            BundleAccount::new(None, Some(info), Default::default(), AccountStatus::Changed),
        );

        let entries = collect_block_bytecodes(empty(), &CacheState::default(), &bundle).unwrap();

        assert_eq!(entries, vec![(code_hash, code.original_bytes().to_vec())]);
    }

    #[test]
    fn captures_execution_bundle_contract_missing_from_tracker() {
        let code = Bytecode::new_raw(Bytes::from_static(&[0x60, 0x2a, 0x60, 0x00, 0x52]));
        let code_hash = code.hash_slow();
        let mut bundle = BundleState::default();
        bundle.contracts.insert(code_hash, code.clone());

        let entries = collect_block_bytecodes(empty(), &CacheState::default(), &bundle).unwrap();

        assert_eq!(entries, vec![(code_hash, code.original_bytes().to_vec())]);
    }

    #[test]
    fn rejects_account_attached_code_with_wrong_hash() {
        let code = Bytecode::new_raw(Bytes::from_static(&[0x60, 0x00, 0x56]));
        let info = AccountInfo {
            balance: U256::ZERO,
            nonce: 1,
            code_hash: B256::repeat_byte(0x11),
            code: Some(code),
        };
        let mut bundle = BundleState::default();
        bundle.state.insert(
            Address::repeat_byte(0x55),
            BundleAccount::new(None, Some(info), Default::default(), AccountStatus::Changed),
        );

        let err = collect_block_bytecodes(empty(), &CacheState::default(), &bundle).unwrap_err();

        assert!(err.to_string().contains("bytecode hash mismatch"));
    }

    #[test]
    fn ignores_empty_bytecode_sentinels() {
        let empty_code = Bytecode::default();
        let mut execution_cache = CacheState::default();
        execution_cache
            .contracts
            .insert(B256::ZERO, empty_code.clone());
        execution_cache
            .contracts
            .insert(KECCAK256_EMPTY, empty_code);

        let entries =
            collect_block_bytecodes(empty(), &execution_cache, &BundleState::default()).unwrap();

        assert!(entries.is_empty());
    }
}
