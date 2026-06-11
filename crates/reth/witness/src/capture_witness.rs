//! Synchronous per-block proof-witness capture via reth's execution-witness path.
//!
//! Produces a reth [`ExecutionWitness`] for the block — the library path under
//! `debug_executionWitness`: re-execute the block against the parent state with
//! reth's [`ExecutionWitnessRecord`], then `StateProvider::witness` for the
//! trie nodes — and builds the per-block [`EvmPartialState`] from it directly
//! via rsp's [`EthereumState::from_execution_witness`]. No hand-rolled
//! multiproof, `CacheDBProvider`, or `from_transition_proofs`.
//!
//! Re-execution runs against the **parent** state (the committed tip at
//! production time, depth 1), so no historical provider is ever opened.

use std::collections::BTreeMap;

use alloy_consensus::Header;
use alloy_primitives::{keccak256, B256};
use alloy_rpc_types_debug::ExecutionWitness;
use reth_evm::{
    execute::{BasicBlockExecutor, Executor},
    ConfigureEvm,
};
use reth_primitives::{Block, EthPrimitives};
use reth_primitives_traits::Block as _;
use reth_provider::{BlockReader, HeaderProvider, StateProofProvider, StateProviderFactory};
use reth_revm::{database::StateProviderDatabase, db::State, state::Bytecode, witness::ExecutionWitnessRecord};
use reth_trie::TrieInput;
use rsp_mpt::EthereumState;
use strata_evm_ee::EvmPartialState;

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
/// block is re-executed against the parent state with reth's
/// [`ExecutionWitnessRecord`]; `StateProvider::witness` then yields the trie
/// nodes, and rsp's [`EthereumState::from_execution_witness`] reconstructs the
/// sparse state directly from that witness. The witness is anchored at the
/// parent state root; the guest's `update()` advances it to the block's root.
///
/// CPU-heavy (re-execution + witness build); call inside
/// [`tokio::task::spawn_blocking`].
pub fn capture_block_witness<P, E>(
    provider: P,
    evm_config: E,
    block_hash: B256,
) -> eyre::Result<CapturedBlockWitness>
where
    P: StateProviderFactory + BlockReader<Block = Block> + HeaderProvider<Header = Header>,
    E: ConfigureEvm<Primitives = EthPrimitives>,
{
    let block = provider
        .block_by_hash(block_hash)?
        .ok_or_else(|| eyre::eyre!("block {block_hash} not found for witness capture"))?;
    let block_num = block.header.number;
    let block_rlp = alloy_rlp::encode(&block);
    let recovered = block.seal_slow().try_recover()?;

    let parent_num = block_num.saturating_sub(1);
    let parent_block = provider
        .block_by_number(parent_num)?
        .ok_or_else(|| eyre::eyre!("parent block {parent_num} not found for witness capture"))?;
    let parent_state_root = parent_block.header.state_root;
    let parent_header_rlp = alloy_rlp::encode(&parent_block.header);

    let state_provider = provider.history_by_block_number(parent_num)?;

    // Re-execute against the parent state, recording the execution witness with
    // reth's maintained recorder. Scoped so the executor's borrow of
    // `state_provider` is released before the `witness()` call below.
    let (hashed_state, codes, lowest_block_number) = {
        let db = StateProviderDatabase::new(&state_provider);
        let executor = BasicBlockExecutor::new(evm_config, db);
        let mut record = ExecutionWitnessRecord::default();
        executor
            .execute_with_state_closure(&recovered, |statedb: &State<_>| {
                record.record_executed_state(statedb);
            })
            .map_err(|e| eyre::eyre!("block re-execution for witness failed: {e}"))?;
        let ExecutionWitnessRecord { hashed_state, codes, lowest_block_number, .. } = record;
        (hashed_state, codes, lowest_block_number)
    };

    // Trie nodes covering the block's touched paths (against the parent state).
    let state = state_provider.witness(TrieInput::default(), hashed_state)?;
    let witness = ExecutionWitness { state, ..Default::default() };

    // rsp builds the sparse state directly from the witness node bag.
    let ethereum_state = EthereumState::from_execution_witness(&witness, parent_state_root);

    // Bytecodes the block loaded, keyed by code hash.
    let bytecodes: BTreeMap<B256, Bytecode> = codes
        .iter()
        .map(|code| (keccak256(code), Bytecode::new_raw(code.clone())))
        .collect();

    // BLOCKHASH ancestor headers: the contiguous range from the lowest block
    // referenced (or just the parent) up to the parent.
    let smallest = lowest_block_number.unwrap_or_else(|| block_num.saturating_sub(1));
    let ancestor_headers: Vec<Header> = provider.headers_range(smallest..block_num)?;

    let partial_state = EvmPartialState::new(ethereum_state, bytecodes, ancestor_headers);

    Ok(CapturedBlockWitness {
        partial_state,
        block_rlp,
        parent_header_rlp,
    })
}
