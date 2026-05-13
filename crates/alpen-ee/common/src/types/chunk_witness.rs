//! Pre-computed witness data for a chunk proof.
//!
//! Written at chunk-seal time by the batch builder (when state is at-tip
//! and historical depth is shallow), read at proof-time by
//! `ChunkSpec::fetch_input` instead of re-running
//! `RangeWitnessExtractor`. This is the storage payload behind the
//! phase 1 redesign documented in
//! `experimental/evgeniy/ee-prover-fetch-input-redesign.md`.
//!
//! The fields are pre-encoded to keep the on-disk type Borsh-friendly:
//! the partial pre-state uses the same codec encoding the chunk guest
//! consumes, and headers / blocks are RLP-encoded (alloy's network
//! encoding) so they round-trip into alloy types at read time.

use borsh::{BorshDeserialize, BorshSerialize};
use strata_acct_types::Hash;

/// Persisted pre-computed witness for one chunk.
///
/// Lifecycle:
/// - **Written** by the batch builder at chunk-seal time, keyed by [`ChunkId`].
/// - **Read** by `ChunkSpec::fetch_input` to assemble the chunk proof input.
/// - **Deleted** on reorg of any contained block, or on chunk cleanup post-finalization.
///
/// Replaces the runtime output of
/// `alpen_reth_witness::RangeWitnessExtractor::extract_range_witness`.
///
/// [`ChunkId`]: super::chunk::ChunkId
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct ChunkWitnessRecord {
    /// Codec-encoded `EvmPartialState` covering accounts/slots/bytecodes
    /// touched by the chunk's blocks, with proofs against pre- and
    /// post-state roots. Same encoding the chunk guest expects in
    /// `PrivateInput::raw_partial_pre_state`.
    pub raw_partial_pre_state: Vec<u8>,

    /// RLP-encoded alloy `Header` for the block immediately preceding the
    /// chunk's first block. Decoded at read time into `EvmHeader`.
    pub prev_header_rlp: Vec<u8>,

    /// RLP-encoded alloy `Block` per chunk block, in chunk order.
    /// Decoded at read time into `EvmBlock`s for `RawBlockData` assembly.
    pub blocks_rlp: Vec<Vec<u8>>,
}

impl ChunkWitnessRecord {
    pub fn new(
        raw_partial_pre_state: Vec<u8>,
        prev_header_rlp: Vec<u8>,
        blocks_rlp: Vec<Vec<u8>>,
    ) -> Self {
        Self {
            raw_partial_pre_state,
            prev_header_rlp,
            blocks_rlp,
        }
    }

    /// Block count in this chunk's witness — convenience for the consumer
    /// to sanity-check against the `Chunk`'s block list.
    pub fn block_count(&self) -> usize {
        self.blocks_rlp.len()
    }
}

/// Sync callback that extracts a chunk witness for the block range
/// `(prev_block, last_block]`.
///
/// Defined as a sync closure (matching today's `RangeWitnessFn` pattern in
/// the alpen-client prover) so callers control whether to wrap it in
/// `tokio::task::spawn_blocking`. Implementations are expected to be
/// CPU-heavy: re-executing the chunk's blocks, computing multiproofs.
///
/// The producer (batch builder) holds an `Arc<ChunkWitnessExtractFn>` and
/// invokes it at chunk-seal time. The closure is constructed at
/// alpen-client startup from the reth `ProviderFactory` + `EvmConfig` and
/// wraps `alpen_reth_witness::RangeWitnessExtractor` plus the alloy-types
/// → `ChunkWitnessRecord` conversion.
pub type ChunkWitnessExtractFn =
    dyn Fn(Hash, Hash) -> eyre::Result<ChunkWitnessRecord> + Send + Sync;
