//! Replay-mode payload builder for the fullnode re-execution ExEx.
//!
//! On the sequencer, [`PayloadBuilderEngine::build_payload`] triggers Reth to
//! build a new block and produces an [`AlpenBuiltPayload`]. On a fullnode the
//! canonical next block has already been imported via p2p; there is no new
//! block to build. This engine reconstructs an equivalent [`AlpenBuiltPayload`]
//! from Reth's already-persisted block and receipts so that the shared
//! assembly core ([`alpen_ee_block_assembly::assemble_next_exec_block_record`])
//! can be reused verbatim between the two paths.
//!
//! ## Scope of "equivalence"
//!
//! The reconstructed payload matches the sequencer-built payload in the
//! fields the assembly pipeline actually consumes — blockhash, withdrawal
//! intents — and is self-consistent for [`EnginePayload::to_bytes`] round-
//! tripping. Byte-for-byte identity with the sequencer's serialized payload
//! is NOT guaranteed yet: `fees`, `requests`, and `payload_id` are set to
//! deterministic placeholders rather than being re-derived from execution
//! state. The fullnode stores payload bytes but does not re-submit them to
//! Reth on startup (unlike the sequencer's `sync_chainstate_to_engine` path),
//! so byte-identity is not load-bearing. Tightening this is a follow-up if a
//! consumer ever relies on cross-node byte equivalence.

use std::sync::Arc;

use alloy_primitives::{B256, U256};
use alloy_rpc_types_engine::{ForkchoiceState, PayloadId};
use alpen_ee_common::{
    ExecutionEngine, ExecutionEngineError, PayloadBuildAttributes, PayloadBuilderEngine,
};
use alpen_reth_evm::extract_withdrawal_intents;
use alpen_reth_node::AlpenBuiltPayload;
use async_trait::async_trait;
use eyre::{eyre, Context};
use reth_ethereum_engine_primitives::{BlobSidecars, EthBuiltPayload};
use reth_primitives::{Receipt, SealedBlock, TransactionSigned};
use reth_provider::{BlockHashReader, BlockNumReader, BlockReader, ReceiptProvider};

/// Reconstructs [`AlpenBuiltPayload`]s from blocks Reth has already imported.
///
/// Generic over any provider that reads canonical blocks and receipts; on the
/// fullnode this is `ctx.provider().clone()` inside the re-execution ExEx.
#[derive(Debug, Clone)]
pub struct RethReplayPayloadEngine<P> {
    provider: P,
}

impl<P> RethReplayPayloadEngine<P> {
    pub fn new(provider: P) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl<P> ExecutionEngine for RethReplayPayloadEngine<P>
where
    P: Send + Sync + 'static,
{
    type TEnginePayload = AlpenBuiltPayload;

    /// No-op: Reth already holds the canonical block via p2p. The fullnode
    /// has no authoritative submission path.
    async fn submit_payload(
        &self,
        _payload: AlpenBuiltPayload,
    ) -> Result<(), ExecutionEngineError> {
        Ok(())
    }

    /// No-op: forkchoice is driven by the engine-control task from the OL
    /// tracker's consensus heads, not by this replay engine.
    async fn update_consensus_state(
        &self,
        _state: ForkchoiceState,
    ) -> Result<(), ExecutionEngineError> {
        Ok(())
    }
}

#[async_trait]
impl<P> PayloadBuilderEngine for RethReplayPayloadEngine<P>
where
    P: BlockReader<Block = reth_ethereum_primitives::Block, Receipt = Receipt>
        + ReceiptProvider<Receipt = Receipt>
        + BlockNumReader
        + BlockHashReader
        + Send
        + Sync
        + 'static,
{
    async fn build_payload(
        &self,
        build_attrs: PayloadBuildAttributes,
    ) -> eyre::Result<AlpenBuiltPayload> {
        let parent_hash = B256::from_slice(build_attrs.parent().as_slice());

        let parent_number = self
            .provider
            .block_number(parent_hash)
            .context("replay_payload_engine: parent block lookup failed")?
            .ok_or_else(|| eyre!("replay_payload_engine: parent block {parent_hash} unknown"))?;

        let child_number = parent_number + 1;

        let child_hash = self
            .provider
            .block_hash(child_number)
            .context("replay_payload_engine: child hash lookup failed")?
            .ok_or_else(|| {
                eyre!("replay_payload_engine: canonical block {child_number} not imported yet")
            })?;

        let recovered_block = self
            .provider
            .recovered_block(child_hash.into(), Default::default())
            .context("replay_payload_engine: recovered block lookup failed")?
            .ok_or_else(|| eyre!("replay_payload_engine: block {child_hash} not recovered"))?;

        let receipts = self
            .provider
            .receipts_by_block(child_hash.into())
            .context("replay_payload_engine: receipts lookup failed")?
            .ok_or_else(|| eyre!("replay_payload_engine: receipts missing for {child_hash}"))?;

        let sealed_block = recovered_block.into_sealed_block();

        Ok(assemble_replay_payload(sealed_block, &receipts))
    }
}

/// Assemble the replay-mode [`AlpenBuiltPayload`] from a sealed block and its
/// receipts. Pure — factored out so the withdrawal-intent extraction and the
/// placeholder (`fees` / `requests` / `payload_id`) construction can be unit-
/// tested without a Reth provider.
pub(crate) fn assemble_replay_payload(
    sealed_block: SealedBlock,
    receipts: &[Receipt],
) -> AlpenBuiltPayload {
    // Extract Alpen's bridge-out withdrawal intents from the already-
    // executed transactions + receipts. Must match what the sequencer stored
    // so the record's withdrawal-intent view lines up.
    let transactions: Vec<TransactionSigned> = sealed_block.body().transactions.to_vec();
    let withdrawal_intents: Vec<_> = extract_withdrawal_intents(&transactions, receipts).collect();

    // `fees`, `requests`, and `payload_id` are placeholders on the replay
    // path — see the module docs. Self-consistent for serialization but not
    // byte-identical to the sequencer's payload.
    let eth_payload = EthBuiltPayload::new(
        PayloadId::new([0u8; 8]),
        Arc::new(sealed_block),
        U256::ZERO,
        None,
    )
    .with_sidecars(BlobSidecars::Empty);

    AlpenBuiltPayload::new(eth_payload, withdrawal_intents)
}

#[cfg(test)]
mod tests {
    use alloy_consensus::Header;
    use alpen_ee_common::EnginePayload;
    use reth_primitives::Block;
    use reth_primitives_traits::Block as BlockTrait;

    use super::*;

    fn empty_sealed_block(number: u64, timestamp: u64) -> SealedBlock {
        let block = Block {
            header: Header {
                number,
                timestamp,
                ..Default::default()
            },
            ..Default::default()
        };
        block.seal_slow()
    }

    #[test]
    fn empty_block_produces_empty_withdrawal_intents() {
        // A block with no transactions has no bridge-out events, so the replay
        // payload's withdrawal-intent view must be empty. This is the common
        // case for a fullnode catching up on quiet epochs; a regression here
        // would surface as the fullnode claiming spurious withdrawals exist.
        let sealed = empty_sealed_block(7, 1_700_000_000);
        let expected_hash = sealed.hash();

        let payload = assemble_replay_payload(sealed, &[]);

        assert!(payload.withdrawal_intents().is_empty());
        // The payload's blockhash must survive assembly unchanged — this is
        // what downstream record construction keys off.
        assert_eq!(payload.blockhash().as_ref(), expected_hash.as_slice());
        assert_eq!(payload.blocknum(), 7);
    }

    #[test]
    fn placeholder_payload_round_trips_through_bytes() {
        // The placeholder `fees` / `requests` / `payload_id` are documented as
        // "not byte-identical to the sequencer" but MUST still be self-
        // consistent — serialize + deserialize must yield the same blockhash
        // and withdrawal intents. Otherwise the fullnode's stored payload
        // bytes would be unreadable by `EnginePayload::from_bytes`, breaking
        // any future consumer (e.g. sync_chainstate_to_engine equivalent).
        let sealed = empty_sealed_block(42, 1_700_000_042);
        let original = assemble_replay_payload(sealed, &[]);
        let bytes = original.to_bytes().expect("serialization succeeds");
        let decoded =
            <AlpenBuiltPayload as EnginePayload>::from_bytes(&bytes).expect("deserialize succeeds");

        assert_eq!(decoded.blockhash(), original.blockhash());
        assert_eq!(decoded.blocknum(), original.blocknum());
        assert_eq!(
            decoded.withdrawal_intents().len(),
            original.withdrawal_intents().len(),
        );
    }
}
