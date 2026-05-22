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

use std::sync::Arc;

use alloy_eips::BlockNumHash;
use alloy_primitives::B256;
use alpen_ee_common::{AccessedAccount, AccessedStateRecord, AccessedStateStore};
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
use reth_revm::{db::CacheDB, state::Bytecode};
use strata_acct_types::Hash;
use tokio::task;
use tracing::{debug, error, warn};

use crate::CacheDBProvider;

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

    let executor = BasicBlockExecutor::new(evm_config, cache_db);
    let _output = executor.execute(&recovered)?;

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
    accounts.sort_by(|a, b| a.address.cmp(&b.address));

    let mut bytecode_hashes: Vec<[u8; 32]> = accessed
        .accessed_contracts()
        .keys()
        .map(|hash| hash.0)
        .collect();
    bytecode_hashes.sort();

    let mut ancestor_block_numbers: Vec<u64> =
        accessed.accessed_block_idxs().iter().copied().collect();
    ancestor_block_numbers.sort();

    let record = AccessedStateRecord {
        accounts,
        bytecode_hashes,
        ancestor_block_numbers,
    };

    let bytecodes = bytecode_entries_from_accessed_contracts(accessed.accessed_contracts().iter());

    Ok((record, bytecodes))
}

fn bytecode_entries_from_accessed_contracts<'a>(
    bytecodes: impl Iterator<Item = (&'a B256, &'a Bytecode)>,
) -> Vec<BytecodeEntry> {
    bytecodes
        .map(|(hash, code)| (hash_from_b256(*hash), code.original_bytes().to_vec()))
        .collect()
}

fn hash_from_b256(hash: B256) -> Hash {
    Hash::from(hash.0)
}

#[cfg(test)]
mod tests {
    use std::iter::once;

    use alloy_primitives::Bytes;

    use super::*;

    #[test]
    fn bytecode_entries_preserve_original_runtime_bytes() {
        let code_hash = B256::from([0x12; 32]);
        let runtime = Bytes::from_static(&[0x60, 0x01, 0x5f, 0x55]);
        let bytecode = Bytecode::new_raw(runtime.clone());

        assert_eq!(bytecode.original_bytes(), runtime);
        assert_ne!(bytecode.bytes(), runtime);

        let entries = bytecode_entries_from_accessed_contracts(once((&code_hash, &bytecode)));

        assert_eq!(entries, vec![(hash_from_b256(code_hash), runtime.to_vec())]);
    }
}
