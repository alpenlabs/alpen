//! Synchronous per-block proof-witness capture.
//!
//! Re-executes a block against its parent state (the committed tip at
//! production time, depth 1) to recover the accessed set and the write set,
//! then builds the block's depth-0 transition witness via
//! [`alpen_reth_witness::build_block_witness`]. Unlike the accessed-state exex
//! this is meant to run **inline** in the block-production path, so a block is
//! never accepted without its witness — there is no separate schedule that can
//! lag tip and push the multiproof to historical depth.
//!
//! Alongside the witness the capture returns the RLP-encoded block and its
//! parent header, so the chunk prover can assemble its input entirely from
//! per-block records without re-touching reth at proof time.

use std::collections::BTreeMap;

use alloy_consensus::Header;
use alloy_primitives::{map::HashMap, Address, B256};
use reth_evm::{
    execute::{BasicBlockExecutor, Executor},
    ConfigureEvm,
};
use reth_primitives::{Block, EthPrimitives};
use reth_primitives_traits::Block as _;
use reth_provider::{BlockReader, StateProviderFactory};
use reth_revm::{db::CacheDB, state::Bytecode};
use reth_trie::HashedPostState;
use reth_trie_common::KeccakKeyHasher;
use strata_evm_ee::EvmPartialState;

use crate::{build_block_witness, CacheDBProvider};

/// Everything the chunk prover needs for one block, captured at production.
#[expect(
    missing_debug_implementations,
    reason = "EvmPartialState inner types don't implement Debug"
)]
pub struct CapturedBlockWitness {
    /// Depth-0 transition witness ([`EvmPartialState`]) for the block.
    pub partial_state: EvmPartialState,
    /// RLP-encoded reth [`Block`] (header + body) for guest re-execution.
    pub block_rlp: Vec<u8>,
    /// RLP-encoded parent [`Header`] (anchors the block's pre-state root).
    pub parent_header_rlp: Vec<u8>,
}

/// Builds the depth-0 proof witness for the block identified by `block_hash`.
///
/// `provider` must see the block, its parent block, and the parent's state. The
/// block is re-executed against the parent state wrapped in a
/// [`CacheDBProvider`] to record the accessed set; the executor output yields
/// the write set. Both the pre- and post-state multiproofs inside
/// [`build_block_witness`] run against the parent provider (depth 0–1 at
/// production time), so no historical state provider is opened.
///
/// CPU-heavy (re-execution + multiproofs); call inside
/// [`tokio::task::spawn_blocking`].
pub fn capture_block_witness<P, E>(
    provider: P,
    evm_config: E,
    block_hash: B256,
) -> eyre::Result<CapturedBlockWitness>
where
    P: StateProviderFactory + BlockReader<Block = Block>,
    E: ConfigureEvm<Primitives = EthPrimitives> + Clone,
{
    let block = provider
        .block_by_hash(block_hash)?
        .ok_or_else(|| eyre::eyre!("block {block_hash} not found for witness capture"))?;
    let block_num = block.header.number;
    let block_rlp = alloy_rlp::encode(&block);
    let sealed = block.seal_slow();
    let recovered = sealed.try_recover()?;

    let parent_num = block_num.saturating_sub(1);
    let parent_block = provider
        .block_by_number(parent_num)?
        .ok_or_else(|| eyre::eyre!("parent block {parent_num} not found for witness capture"))?;
    let start_state_root = parent_block.header.state_root;
    let parent_header_rlp = alloy_rlp::encode(&parent_block.header);

    // Re-execute the block against the parent state with a recording DB to
    // recover the accessed set and the write set.
    let history = provider.history_by_block_number(parent_num)?;
    let cache_provider = CacheDBProvider::new(history);
    let cache_db = CacheDB::new(&cache_provider);
    let executor = BasicBlockExecutor::new(evm_config, cache_db);
    let output = executor.execute(&recovered)?;

    let accessed = cache_provider.get_accessed_state();

    // Touched accounts -> their accessed storage slots.
    let touched: HashMap<Address, Vec<B256>> = accessed
        .accessed_accounts()
        .iter()
        .map(|(addr, slots)| {
            (
                *addr,
                slots
                    .iter()
                    .map(|slot| B256::from(slot.to_be_bytes::<32>()))
                    .collect::<Vec<_>>(),
            )
        })
        .collect();

    let bytecodes: BTreeMap<B256, Bytecode> = accessed
        .accessed_contracts()
        .iter()
        .map(|(hash, code)| (*hash, code.clone()))
        .collect();

    // BLOCKHASH ancestor headers actually used by the block.
    let mut ancestor_headers: Vec<Header> = Vec::new();
    for idx in accessed.accessed_block_idxs() {
        let header = provider
            .block_by_number(*idx)?
            .ok_or_else(|| eyre::eyre!("ancestor block {idx} not found for witness capture"))?
            .header;
        ancestor_headers.push(header);
    }

    // The block's write set, used as the post-state overlay so the post
    // multiproof yields proofs valid at `root(block_num)`.
    let write_set = HashedPostState::from_bundle_state::<KeccakKeyHasher>(&output.state.state);

    // A clean parent state provider for the multiproofs — the one above is
    // consumed by re-execution.
    let mp_provider = provider.history_by_block_number(parent_num)?;
    let partial_state = build_block_witness(
        &mp_provider,
        &touched,
        write_set,
        start_state_root,
        bytecodes,
        ancestor_headers,
    )?;

    Ok(CapturedBlockWitness {
        partial_state,
        block_rlp,
        parent_header_rlp,
    })
}
