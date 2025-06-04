use std::{collections::HashSet, sync::Arc};

use alloy_consensus::{BlockHeader, Header};
use alloy_primitives::map::foldhash::{HashMap, HashMapExt};
use alloy_rpc_types::BlockNumHash;
use alpen_reth_db::WitnessStore;
use eyre::eyre;
use futures_util::TryStreamExt;
use reth_chainspec::EthChainSpec;
use reth_evm::execute::{BlockExecutorProvider, Executor};
use reth_exex::{ExExContext, ExExEvent};
use reth_node_api::{Block as _, FullNodeComponents, NodeTypes};
use reth_primitives::EthPrimitives;
use reth_provider::{BlockReader, Chain, ExecutionOutcome, StateProvider, StateProviderFactory};
use reth_revm::{db::CacheDB, primitives::FixedBytes};
use reth_trie::{HashedPostState, TrieInput};
use reth_trie_common::KeccakKeyHasher;
use revm_primitives::alloy_primitives::B256;
use rsp_mpt::EthereumState;
use strata_proofimpl_evm_ee_stf::EvmBlockStfInput;
use tracing::{debug, error};

use crate::{
    alloy2reth::IntoRspGenesis,
    cache_db_provider::{AccessedState, CacheDBProvider},
};

#[allow(missing_debug_implementations)]
pub struct ProverWitnessGenerator<
    Node: FullNodeComponents<Types: NodeTypes<Primitives = EthPrimitives>>,
    S: WitnessStore + Clone,
> {
    ctx: ExExContext<Node>,
    db: Arc<S>,
}

impl<
        Node: FullNodeComponents<Types: NodeTypes<Primitives = EthPrimitives>>,
        S: WitnessStore + Clone,
    > ProverWitnessGenerator<Node, S>
{
    pub fn new(ctx: ExExContext<Node>, db: Arc<S>) -> Self {
        Self { ctx, db }
    }

    fn commit(&mut self, chain: &Chain) -> eyre::Result<Option<BlockNumHash>> {
        let mut finished_height = None;
        let blocks = chain.blocks();
        let bundles = chain.range().filter_map(|block_number| {
            blocks
                .get(&block_number)
                .map(|block| block.hash())
                .zip(chain.execution_outcome_at_block(block_number))
        });

        for (block_hash, outcome) in bundles {
            #[cfg(debug_assertions)]
            assert!(outcome.len() == 1, "should only contain single block");

            let prover_input = extract_zkvm_input(block_hash, &self.ctx, &outcome)?;

            // TODO: maybe put db writes in another thread
            if let Err(err) = self.db.put_block_witness(block_hash, &prover_input) {
                error!(?err, ?block_hash);
                break;
            }

            finished_height = Some(BlockNumHash::new(outcome.first_block(), block_hash))
        }

        Ok(finished_height)
    }

    pub async fn start(mut self) -> eyre::Result<()> {
        debug!("start prover witness generator");
        while let Some(notification) = self.ctx.notifications.try_next().await? {
            if let Some(committed_chain) = notification.committed_chain() {
                let finished_height = self.commit(&committed_chain)?;
                if let Some(finished_height) = finished_height {
                    self.ctx
                        .events
                        .send(ExExEvent::FinishedHeight(finished_height))?;
                }
            }
        }

        Ok(())
    }
}

fn extract_zkvm_input<Node>(
    block_id: FixedBytes<32>,
    ctx: &ExExContext<Node>,
    exec_outcome: &ExecutionOutcome,
) -> eyre::Result<EvmBlockStfInput>
where
    Node: FullNodeComponents,
    Node::Types: NodeTypes<Primitives = EthPrimitives>,
{
    let genesis = ctx.config.chain.genesis().clone().try_into_rsp()?;

    // fetch current block
    let current_block = ctx
        .provider()
        .block_by_hash(block_id)?
        .ok_or_else(|| eyre!("block not found for hash {:?}", block_id))?;
    let current_block_idx = current_block.number;

    // fetch previous block
    let prev_block_idx = current_block_idx - 1;
    let prev_block = ctx
        .provider()
        .block_by_number(prev_block_idx)?
        .ok_or_else(|| eyre!("previous block not found for number {}", prev_block_idx))?;
    let prev_block_stateroot = prev_block.header.state_root;

    // execute to collect accessed state
    let accessed_info = get_accessed_states(ctx, block_id)?;
    let accessed_ancestors =
        get_ancestor_headers(ctx, current_block_idx, accessed_info.accessed_block_idxs())?;

    let parent_state = derive_parent_state(
        ctx.provider()
            .history_by_block_number(current_block_idx - 1)?,
        prev_block_stateroot,
        &accessed_info,
        exec_outcome,
    )?;

    Ok(EvmBlockStfInput {
        genesis,
        current_block,
        parent_state,
        ancestor_headers: accessed_ancestors,
        state_requests: accessed_info.accessed_accounts().clone(),
        bytecodes: accessed_info.accessed_contracts().clone(),
        custom_beneficiary: None,
        opcode_tracking: false,
    })
}

fn derive_parent_state<P>(
    provider: P,
    start_state_root: FixedBytes<32>,
    accessed_states: &AccessedState,
    exec_outcome: &ExecutionOutcome,
) -> eyre::Result<EthereumState>
where
    P: StateProvider,
{
    let mut before_proofs = HashMap::new();
    let mut after_proofs = HashMap::new();

    // Iterate through accessed accounts
    for (address, slots) in accessed_states.accessed_accounts().iter() {
        // Convert slots to keys
        let keys = slots
            .iter()
            .map(|slot| B256::from_slice(&slot.to_be_bytes::<32>()))
            .collect::<Vec<_>>();

        // Get proof before execution
        let root_before = HashedPostState::from_bundle_state::<KeccakKeyHasher>([]);
        let proof_before = provider.proof(TrieInput::from_state(root_before), *address, &keys)?;

        // Get proof after execution
        let root_after = exec_outcome.hash_state_slow::<KeccakKeyHasher>();
        let proof_after = provider.proof(TrieInput::from_state(root_after), *address, &keys)?;

        // Store proofs in the maps
        before_proofs.insert(*address, proof_before);
        after_proofs.insert(*address, proof_after);
    }

    let parent_state =
        EthereumState::from_transition_proofs(start_state_root, &before_proofs, &after_proofs)?;

    Ok(parent_state)
}

fn get_accessed_states<Node>(
    ctx: &ExExContext<Node>,
    block_id: FixedBytes<32>,
) -> eyre::Result<AccessedState>
where
    Node: FullNodeComponents,
    Node::Types: NodeTypes<Primitives = EthPrimitives>,
{
    // fetch the block header by hash
    let header_block = ctx
        .provider()
        .block_by_hash(block_id)?
        .ok_or_else(|| eyre!("block not found for hash {:?}", block_id))?;
    let block_number = header_block.number();

    // recover the execution input
    let recovered = header_block
        .clone()
        .seal_unchecked(block_id)
        .try_recover()?;

    // look up the history provider for the parent block
    let parent_number = block_number
        .checked_sub(1)
        .ok_or_else(|| eyre!("no parent block for block {}", block_number))?;
    let history_provider = ctx.provider().history_by_block_number(parent_number)?;

    // wrap in a cache-backed provider and run the executor
    let cache_provider = CacheDBProvider::new(history_provider);
    let cache_db = CacheDB::new(&cache_provider);
    ctx.block_executor()
        .clone()
        .executor(cache_db)
        .execute(&recovered)?;

    Ok(cache_provider.get_accessed_state())
}

fn get_ancestor_headers<Node>(
    ctx: &ExExContext<Node>,
    current_idx: u64,
    accessed_idxs: &HashSet<u64>,
) -> eyre::Result<Vec<Header>>
where
    Node: FullNodeComponents,
    Node::Types: NodeTypes<Primitives = EthPrimitives>,
{
    let mut acc = accessed_idxs.clone();
    acc.insert(current_idx - 1);

    // get vec of all sorted accessed block numbers
    let oldest_parent = acc
        .iter()
        .min_by_key(|&&x| x)
        .copied()
        .unwrap_or(current_idx - 1);

    (oldest_parent..current_idx)
        .rev()
        .map(|num| {
            ctx.provider()
                .block_by_number(num)?
                .map(|b| b.header)
                .ok_or_else(|| eyre!("block not found for number {}", num))
        })
        .collect()
}
