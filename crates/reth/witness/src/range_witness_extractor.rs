//! Range witness extraction for arbitrary block ranges.

use std::collections::HashSet;

use alloy_consensus::Header;
use alloy_primitives::{
    keccak256,
    map::{B256Set, DefaultHashBuilder, HashMap},
    Address, B256, U256,
};
use alpen_reth_exex::{AccessedState, CacheDBProvider};
use eyre::{eyre, Result};
use reth_evm::{
    execute::{BasicBlockExecutor, Executor},
    ConfigureEvm,
};
use reth_primitives::EthPrimitives;
use reth_primitives_traits::Block as _;
use reth_provider::{BlockReader, StateProvider, StateProviderFactory};
use reth_revm::{db::CacheDB, state::Bytecode};
use reth_trie::{HashedPostState, MultiProofTargets, TrieInput};
use reth_trie_common::KeccakKeyHasher;
use rsp_mpt::EthereumState;
use strata_codec::encode_to_vec;
use strata_evm_ee::EvmPartialState;
use tracing::debug;

/// Witness data extracted for a block range.
#[derive(Debug)]
pub struct RangeWitnessData {
    pub start_block_hash: B256,
    pub end_block_hash: B256,
    /// Serialized `EvmPartialState` (via `strata_codec`).
    pub raw_partial_pre_state: Vec<u8>,
    pub raw_prev_header: Vec<u8>,
}

/// Extracts witness data for block ranges.
#[derive(Debug)]
pub struct RangeWitnessExtractor<F, E> {
    provider_factory: F,
    evm_config: E,
}

impl<F, E> RangeWitnessExtractor<F, E>
where
    F: StateProviderFactory + BlockReader<Block = reth_primitives::Block>,
    E: ConfigureEvm<Primitives = EthPrimitives> + Clone,
{
    pub fn new(provider_factory: F, evm_config: E) -> Self {
        Self {
            provider_factory,
            evm_config,
        }
    }

    /// Extracts witness for the block range `[start_block_hash, end_block_hash]` (inclusive).
    pub fn extract_range_witness(
        &self,
        start_block_hash: B256,
        end_block_hash: B256,
    ) -> Result<RangeWitnessData> {
        // Resolve hashes to blocks
        let start_block = self
            .provider_factory
            .block_by_hash(start_block_hash)?
            .ok_or_else(|| eyre!("start block not found for hash {}", start_block_hash))?;
        let end_block = self
            .provider_factory
            .block_by_hash(end_block_hash)?
            .ok_or_else(|| eyre!("end block not found for hash {}", end_block_hash))?;

        let start_block_num = start_block.number;
        let end_block_num = end_block.number;

        debug!(start_block_num, end_block_num, %start_block_hash, %end_block_hash, "extracting range witness");

        // Fetch previous block using parent hash
        let prev_block_hash = start_block.header.parent_hash;
        let prev_block = self
            .provider_factory
            .block_by_hash(prev_block_hash)?
            .ok_or_else(|| eyre!("previous block not found for hash {}", prev_block_hash))?;
        let prev_block_num = prev_block.number;
        let start_state_root = prev_block.header.state_root;

        // 1. Execute all blocks to discover accessed state
        let accessed = self.execute_blocks_for_accessed_state(start_block_num, end_block_num)?;

        // 2. Get providers for pre-range and post-range states
        let pre_state_provider = self
            .provider_factory
            .history_by_block_number(prev_block_num)?;
        let post_state_provider = self
            .provider_factory
            .history_by_block_number(end_block_num)?;

        // 3. Generate multiproofs for all accessed accounts
        let (ethereum_state, bytecodes) = self.build_ethereum_state(
            &pre_state_provider,
            &post_state_provider,
            start_state_root,
            &accessed,
        )?;

        // 4. Get ancestor headers for BLOCKHASH opcode
        let ancestor_headers = self.get_ancestor_headers(start_block_num, &accessed.block_idxs)?;

        // 5. Build and serialize EvmPartialState
        let partial_state = EvmPartialState::new(ethereum_state, bytecodes, ancestor_headers);
        let raw_partial_pre_state = encode_to_vec(&partial_state)
            .map_err(|e| eyre!("failed to encode partial state: {e}"))?;

        let raw_prev_header = alloy_rlp::encode(&prev_block.header);

        Ok(RangeWitnessData {
            start_block_hash,
            end_block_hash,
            raw_partial_pre_state,
            raw_prev_header,
        })
    }

    fn execute_blocks_for_accessed_state(
        &self,
        start_block: u64,
        end_block: u64,
    ) -> Result<AccumulatedState> {
        let mut acc = AccumulatedState::default();

        for blk_num in start_block..=end_block {
            let block = self
                .provider_factory
                .block_by_number(blk_num)?
                .ok_or_else(|| eyre!("block {} not found", blk_num))?;

            let sealed = block.seal_slow();
            let recovered = sealed.try_recover()?;

            // Get history at parent block for this execution
            let history = self
                .provider_factory
                .history_by_block_number(blk_num.saturating_sub(1))?;
            let cache_provider = CacheDBProvider::new(history);
            let cache_db = CacheDB::new(&cache_provider);

            let executor = BasicBlockExecutor::new(self.evm_config.clone(), cache_db);
            let _output = executor.execute(&recovered)?;

            acc.merge(&cache_provider.get_accessed_state());
        }

        Ok(acc)
    }

    fn build_ethereum_state<P>(
        &self,
        pre_state: &P,
        post_state: &P,
        start_state_root: B256,
        accessed: &AccumulatedState,
    ) -> Result<(EthereumState, Vec<Bytecode>)>
    where
        P: StateProvider,
    {
        // Build touched accounts map: address -> storage keys
        let touched: HashMap<Address, Vec<B256>> = accessed
            .accounts
            .iter()
            .map(|(addr, slots)| {
                let keys = slots
                    .iter()
                    .map(|s| B256::from(s.to_be_bytes::<32>()))
                    .collect();
                (*addr, keys)
            })
            .collect();

        // ALL accessed accounts go into multiproof targets
        let targets = MultiProofTargets::from_iter(touched.iter().map(|(addr, keys)| {
            (
                keccak256(addr),
                B256Set::from_iter(keys.iter().map(keccak256)),
            )
        }));

        // Generate pre-state and post-state multiproofs
        let proof_pre = pre_state.multiproof(
            TrieInput::from_state(HashedPostState::from_bundle_state::<KeccakKeyHasher>([])),
            targets.clone(),
        )?;
        let proof_post = post_state.multiproof(
            TrieInput::from_state(HashedPostState::from_bundle_state::<KeccakKeyHasher>([])),
            targets,
        )?;

        // Extract account proofs, track pre-existing accounts
        let mut pre_proofs =
            HashMap::with_capacity_and_hasher(touched.len(), DefaultHashBuilder::default());
        let mut post_proofs =
            HashMap::with_capacity_and_hasher(touched.len(), DefaultHashBuilder::default());
        let mut accessed_accounts = HashSet::new();

        for (addr, keys) in &touched {
            pre_proofs.insert(*addr, proof_pre.account_proof(*addr, keys)?);
            post_proofs.insert(*addr, proof_post.account_proof(*addr, keys)?);

            // Only accounts existing at (start_block - 1) are "pre-existing"
            if pre_state.basic_account(addr)?.is_some() {
                accessed_accounts.insert(*addr);
            }
        }

        let state =
            EthereumState::from_transition_proofs(start_state_root, &pre_proofs, &post_proofs)?;
        let bytecodes: Vec<Bytecode> = accessed.bytecodes.iter().cloned().collect();
        Ok((state, bytecodes))
    }

    fn get_ancestor_headers(
        &self,
        start_block: u64,
        accessed_idxs: &HashSet<u64>,
    ) -> Result<Vec<Header>> {
        let prev_block = start_block.saturating_sub(1);
        let oldest = accessed_idxs
            .iter()
            .min()
            .copied()
            .unwrap_or(prev_block)
            .min(prev_block);

        (oldest..start_block)
            .rev()
            .map(|n| {
                self.provider_factory
                    .block_by_number(n)?
                    .map(|b| b.header.clone())
                    .ok_or_else(|| eyre!("block {} not found", n))
            })
            .collect()
    }
}

#[derive(Debug, Default)]
struct AccumulatedState {
    accounts: HashMap<Address, HashSet<U256>>,
    bytecodes: HashSet<Bytecode>,
    block_idxs: HashSet<u64>,
}

impl AccumulatedState {
    fn merge(&mut self, other: &AccessedState) {
        for (addr, slots) in other.accessed_accounts() {
            self.accounts
                .entry(*addr)
                .or_default()
                .extend(slots.iter().copied());
        }
        self.bytecodes
            .extend(other.accessed_contracts().iter().cloned());
        self.block_idxs.extend(other.accessed_block_idxs());
    }
}
