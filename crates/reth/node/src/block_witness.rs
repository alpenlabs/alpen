//! Inline per-block proof-witness capture, produced during payload build.
//!
//! Builds the depth-0 transition witness for a freshly built block by reading
//! the access set straight out of the just-executed reth [`State`] — no
//! re-execution. This is the producer side of the inline witness path: capture
//! happens in `try_build_payload` (see [`crate::payload_builder`]) while the
//! block is at tip, so every reth multiproof is at depth 0–1 and no historical
//! provider is ever opened.
//!
//! The resulting [`BlockWitnessRecord`] is the self-contained per-block record
//! the chunk prover assembles its input from, without re-touching reth at proof
//! time.

use std::collections::BTreeMap;

use alloy_consensus::Header;
use alloy_primitives::{keccak256, B256};
use alloy_rpc_types_debug::ExecutionWitness;
use borsh::{BorshDeserialize, BorshSerialize};
use reth_provider::{HeaderProvider, StateProofProvider};
use reth_revm::{db::State, state::Bytecode, witness::ExecutionWitnessRecord, Database};
use reth_trie::TrieInput;
use rsp_mpt::EthereumState;
use strata_codec::encode_to_vec;
use strata_evm_ee::EvmPartialState;

/// Everything the chunk prover needs for one block, captured at production.
#[expect(
    missing_debug_implementations,
    reason = "EvmPartialState inner types don't implement Debug"
)]
pub struct CapturedBlockWitness {
    /// Depth-0 transition witness ([`EvmPartialState`]) for the block.
    pub partial_state: EvmPartialState,
    /// RLP-encoded reth `Block` (header + body) for guest re-execution.
    pub block_rlp: Vec<u8>,
    /// RLP-encoded parent [`Header`] (anchors the block's pre-state root).
    pub parent_header_rlp: Vec<u8>,
}

/// Builds the depth-0 proof witness from an already-executed block state, with
/// **no re-execution**.
///
/// Reads the witness directly out of the `executed_state` produced while the
/// block was built — the live reth [`State`] right after
/// `BlockBuilder::finish`. Reusing that state is the whole point of inline
/// capture: the single production execution both commits state and yields its
/// access set, so no second execution is needed.
///
/// `executed_state` supplies the access set (touched accounts/slots, loaded
/// `codes`, BLOCKHASH range) via reth's [`ExecutionWitnessRecord`].
/// `state_provider` must be the parent state (it serves the depth-0
/// [`StateProofProvider::witness`] trie nodes), and `header_provider` must cover
/// the BLOCKHASH ancestor range. `parent_header` anchors the pre-state root; the
/// guest's `update()` advances it to the block's root.
pub fn build_block_witness_from_executed_state<DB, SP, HP>(
    executed_state: &State<DB>,
    state_provider: &SP,
    header_provider: &HP,
    block_num: u64,
    block_rlp: Vec<u8>,
    parent_header: &Header,
) -> eyre::Result<CapturedBlockWitness>
where
    DB: Database,
    SP: StateProofProvider,
    HP: HeaderProvider<Header = Header>,
{
    // Access set read straight out of the post-execution state — no re-run.
    let mut record = ExecutionWitnessRecord::default();
    record.record_executed_state(executed_state);
    let ExecutionWitnessRecord {
        hashed_state,
        codes,
        lowest_block_number,
        ..
    } = record;

    // Trie nodes covering the block's touched paths (against the parent state).
    let state = state_provider.witness(TrieInput::default(), hashed_state)?;
    let witness = ExecutionWitness {
        state,
        ..Default::default()
    };

    let parent_state_root = parent_header.state_root;
    let ethereum_state = EthereumState::from_execution_witness(&witness, parent_state_root);

    // Bytecodes the block loaded, keyed by code hash.
    let bytecodes: BTreeMap<B256, Bytecode> = codes
        .iter()
        .map(|code| (keccak256(code), Bytecode::new_raw(code.clone())))
        .collect();

    // BLOCKHASH ancestor headers: the contiguous range from the lowest block
    // referenced (or just the parent) up to the parent.
    let smallest = lowest_block_number.unwrap_or_else(|| block_num.saturating_sub(1));
    let ancestor_headers: Vec<Header> = header_provider.headers_range(smallest..block_num)?;

    let parent_header_rlp = alloy_rlp::encode(parent_header);

    let partial_state = EvmPartialState::new(ethereum_state, bytecodes, ancestor_headers);

    Ok(CapturedBlockWitness {
        partial_state,
        block_rlp,
        parent_header_rlp,
    })
}

/// Persisted per-block proof-witness, keyed by execution block hash.
///
/// Self-contained so the chunk prover can assemble a chunk proof input from the
/// chunk's per-block records alone, without re-touching reth at proof time.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct BlockWitnessRecord {
    /// Codec-encoded [`EvmPartialState`] — the block's depth-0 transition
    /// witness, anchored at its parent state root. Same encoding the chunk
    /// guest consumes via `RawBlockData::raw_partial_pre_state`.
    pub raw_partial_pre_state: Vec<u8>,
    /// RLP-encoded reth `Block` (header + body) for guest re-execution.
    pub raw_block_rlp: Vec<u8>,
    /// RLP-encoded parent alloy [`Header`] (anchors the pre-state root). For a
    /// chunk's first block this is the chunk's `prev_header`.
    pub raw_parent_header_rlp: Vec<u8>,
}

impl CapturedBlockWitness {
    /// Encodes this witness into the persisted [`BlockWitnessRecord`] byte form
    /// stored in the block-witness store and consumed by the chunk prover.
    pub fn encode_record(&self) -> eyre::Result<Vec<u8>> {
        let record = BlockWitnessRecord {
            raw_partial_pre_state: encode_to_vec(&self.partial_state)
                .map_err(|e| eyre::eyre!("encode partial state: {e}"))?,
            raw_block_rlp: self.block_rlp.clone(),
            raw_parent_header_rlp: self.parent_header_rlp.clone(),
        };
        borsh::to_vec(&record).map_err(|e| eyre::eyre!("encode block witness record: {e}"))
    }
}
