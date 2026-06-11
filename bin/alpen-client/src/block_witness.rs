//! Reth-backed [`BlockWitnessProducer`] implementation and the per-block
//! witness record it persists.
//!
//! Wires the inline per-block witness capture into the block-production path:
//! re-executes the block against its parent state via
//! [`alpen_reth_witness::capture_block_witness`] and persists a
//! [`BlockWitnessRecord`] (the depth-0 partial-state witness plus the RLP block
//! and parent header) to the [`BlockWitnessStore`]. The block builder awaits
//! this before accepting a block, so a block is never advanced without its
//! witness, and the chunk prover assembles its input purely from these records.

use std::sync::Arc;

use alloy_primitives::B256;
use alpen_ee_common::BlockWitnessStore;
use alpen_ee_sequencer::BlockWitnessProducer;
use alpen_reth_witness::capture_block_witness;
use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use reth_evm::ConfigureEvm;
use reth_primitives::{Block, EthPrimitives};
use reth_provider::{BlockReader, StateProviderFactory};
use strata_acct_types::Hash;
use strata_codec::encode_to_vec;
use tokio::task;

/// Persisted per-block proof-witness, keyed by execution block hash.
///
/// Self-contained so the chunk prover can assemble a chunk proof input from the
/// chunk's per-block records alone, without re-touching reth at proof time.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub(crate) struct BlockWitnessRecord {
    /// Codec-encoded `EvmPartialState` — the block's depth-0 transition
    /// witness, anchored at its parent state root. Same encoding the chunk
    /// guest consumes via `RawBlockData::raw_partial_pre_state`.
    pub(crate) raw_partial_pre_state: Vec<u8>,
    /// RLP-encoded reth `Block` (header + body) for guest re-execution.
    pub(crate) raw_block_rlp: Vec<u8>,
    /// RLP-encoded parent alloy `Header` (anchors the pre-state root). For a
    /// chunk's first block this is the chunk's `prev_header`.
    pub(crate) raw_parent_header_rlp: Vec<u8>,
}

/// Reth-backed producer: re-executes a block against its parent state and
/// persists the resulting [`BlockWitnessRecord`].
pub(crate) struct RethBlockWitnessProducer<P, E, S> {
    provider: P,
    evm_config: E,
    store: Arc<S>,
}

impl<P, E, S> RethBlockWitnessProducer<P, E, S> {
    pub(crate) fn new(provider: P, evm_config: E, store: Arc<S>) -> Self {
        Self {
            provider,
            evm_config,
            store,
        }
    }
}

#[async_trait]
impl<P, E, S> BlockWitnessProducer for RethBlockWitnessProducer<P, E, S>
where
    P: StateProviderFactory + BlockReader<Block = Block> + Clone + Send + Sync + 'static,
    E: ConfigureEvm<Primitives = EthPrimitives> + Clone + Send + Sync + 'static,
    S: BlockWitnessStore + 'static,
{
    async fn produce_block_witness(&self, block_hash: Hash) -> eyre::Result<()> {
        let provider = self.provider.clone();
        let evm_config = self.evm_config.clone();
        let block_hash_b256 = B256::from(<[u8; 32]>::from(block_hash));

        // Re-execution + multiproofs are CPU-heavy; run off the async runtime.
        let captured = task::spawn_blocking(move || {
            capture_block_witness(provider, evm_config, block_hash_b256)
        })
        .await
        .map_err(|e| eyre::eyre!("block witness capture join error: {e}"))??;

        let record = BlockWitnessRecord {
            raw_partial_pre_state: encode_to_vec(&captured.partial_state)
                .map_err(|e| eyre::eyre!("encode partial state: {e}"))?,
            raw_block_rlp: captured.block_rlp,
            raw_parent_header_rlp: captured.parent_header_rlp,
        };
        let bytes =
            borsh::to_vec(&record).map_err(|e| eyre::eyre!("encode block witness record: {e}"))?;
        self.store
            .put_block_witness(block_hash, bytes)
            .await
            .map_err(|e| eyre::eyre!("persist block witness: {e}"))?;
        Ok(())
    }
}
