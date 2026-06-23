//! Inline per-block proof-witness capture, produced during payload build.
//!
//! Harvests the raw execution-witness parts for a freshly built block by
//! reading the access set straight out of the just-executed reth [`State`] — no
//! re-execution. This is the producer side of the witness path: capture happens
//! in `try_build_payload` (see [`crate::payload_builder`]) while the block is at
//! tip.
//!
//! Each block's [`BlockWitnessRecord`] stores only the *raw* witness inputs (the
//! trie-node bag, loaded bytecodes, and BLOCKHASH ancestor headers), not a
//! built trie. The chunk prover unions these per-block bags into one chunk-level
//! sparse state at assembly time (see `EvmPartialState::from_witness_parts`), so
//! the trie reconstruction is a chunk-level concern.

use alloy_consensus::Header;
use reth_provider::{HeaderProvider, StateProofProvider};
use reth_revm::{db::State, witness::ExecutionWitnessRecord, Database};
use reth_trie::TrieInput;
use serde::{Deserialize, Serialize};

/// Persisted per-block proof-witness, keyed by execution block hash.
///
/// Holds the raw witness inputs the chunk prover needs to reconstruct state: the
/// trie-node bag (`witness_state`), the bytecodes the block loaded (`codes`),
/// and the BLOCKHASH ancestor headers — plus the RLP block and parent header.
/// The chunk prover unions the node bags across a chunk's blocks and builds one
/// chunk-level sparse state from them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockWitnessRecord {
    /// Bag of RLP-encoded MPT nodes for the block's touched paths, anchored at
    /// the block's parent state root (reth's `ExecutionWitness::state` format).
    pub witness_state: Vec<Vec<u8>>,
    /// Bytecodes the block loaded (raw bytes; keyed by keccak hash downstream).
    pub codes: Vec<Vec<u8>>,
    /// RLP-encoded BLOCKHASH ancestor headers covering the block's range.
    pub ancestor_headers: Vec<Vec<u8>>,
    /// RLP-encoded reth `Block` (header + body) for guest re-execution.
    pub raw_block_rlp: Vec<u8>,
    /// RLP-encoded parent alloy [`Header`] (anchors the block's pre-state root).
    /// For a chunk's first block this is the chunk's `prev_header`.
    pub raw_parent_header_rlp: Vec<u8>,
}

impl BlockWitnessRecord {
    /// Encodes the record to CBOR for storage.
    pub fn encode(&self) -> eyre::Result<Vec<u8>> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf)
            .map_err(|e| eyre::eyre!("cbor encode block witness record: {e}"))?;
        Ok(buf)
    }

    /// Decodes a CBOR-encoded record from storage.
    pub fn decode(bytes: &[u8]) -> eyre::Result<Self> {
        ciborium::from_reader(bytes)
            .map_err(|e| eyre::eyre!("cbor decode block witness record: {e}"))
    }
}

/// Harvests the raw depth-0 witness parts from an already-executed block state,
/// with **no re-execution**.
///
/// Reads the access set directly out of the `executed_state` produced while the
/// block was built — the live reth [`State`] right after `BlockBuilder::finish`.
/// Reusing that state is the whole point of inline capture: the single
/// production execution both commits state and yields its access set.
///
/// `executed_state` supplies the access set (touched accounts/slots, loaded
/// `codes`, BLOCKHASH range) via reth's [`ExecutionWitnessRecord`].
/// `state_provider` must be the parent state (it serves the depth-0
/// [`StateProofProvider::witness`] trie nodes), and `header_provider` must cover
/// the BLOCKHASH ancestor range.
pub fn build_block_witness_from_executed_state<DB, SP, HP>(
    executed_state: &State<DB>,
    state_provider: &SP,
    header_provider: &HP,
    block_num: u64,
    block_rlp: Vec<u8>,
    parent_header: &Header,
) -> eyre::Result<BlockWitnessRecord>
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
    let witness_state = state_provider
        .witness(TrieInput::default(), hashed_state)?
        .into_iter()
        .map(|node| node.to_vec())
        .collect();

    let codes = codes.into_iter().map(|code| code.to_vec()).collect();

    // BLOCKHASH ancestor headers: the contiguous range from the lowest block
    // referenced (or just the parent) up to the parent.
    let smallest = lowest_block_number.unwrap_or_else(|| block_num.saturating_sub(1));
    let ancestor_headers = header_provider
        .headers_range(smallest..block_num)?
        .iter()
        .map(alloy_rlp::encode)
        .collect();

    let raw_parent_header_rlp = alloy_rlp::encode(parent_header);

    Ok(BlockWitnessRecord {
        witness_state,
        codes,
        ancestor_headers,
        raw_block_rlp: block_rlp,
        raw_parent_header_rlp,
    })
}
