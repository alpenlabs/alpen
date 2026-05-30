use std::future::Future;

use alloy_primitives::B256;
use alloy_rpc_types_engine::ForkchoiceState;
use alpen_ee_common::{BlockNumHash, ConsensusHeads, ExecutionEngine};
use reth_node_builder::NodeTypesWithDB;
use reth_provider::{
    providers::{BlockchainProvider, ProviderNodeTypes},
    BlockHashReader, BlockNumReader, ProviderResult,
};
use tokio::{select, sync::watch};
use tracing::{error, warn};

#[cfg_attr(test, mockall::automock)]
trait CanonicalChainReader {
    fn is_in_canonical_chain(&self, blockhash: B256) -> ProviderResult<bool>;
}

struct RethCanonicalChainReader<'a, N: NodeTypesWithDB + ProviderNodeTypes> {
    provider: &'a BlockchainProvider<N>,
}

impl<'a, N: NodeTypesWithDB + ProviderNodeTypes> RethCanonicalChainReader<'a, N> {
    fn new(provider: &'a BlockchainProvider<N>) -> Self {
        Self { provider }
    }
}

impl<'a, N: NodeTypesWithDB + ProviderNodeTypes> CanonicalChainReader
    for RethCanonicalChainReader<'a, N>
{
    fn is_in_canonical_chain(&self, blockhash: B256) -> ProviderResult<bool> {
        let Some(block_number) = self.provider.block_number(blockhash)? else {
            return Ok(false);
        };
        let Some(canonical_blockhash) = self.provider.block_hash(block_number)? else {
            return Ok(false);
        };
        Ok(blockhash == canonical_blockhash)
    }
}

fn forkchoice_state_from_consensus_with_reader(
    consensus_state: &ConsensusHeads,
    head_block_hash: B256,
    reader: &impl CanonicalChainReader,
) -> ProviderResult<ForkchoiceState> {
    let safe_block_hash = B256::from_slice(consensus_state.confirmed().as_slice());
    let finalized_block_hash = B256::from_slice(consensus_state.finalized().as_slice());

    let head_block_hash = if reader.is_in_canonical_chain(safe_block_hash)? {
        head_block_hash
    } else {
        // Safe block is not in canonical chain on reth.
        // This means either:
        // 1. This is during initial sync and OL chain is ahead of reth
        // 2. There is a fork
        // In either case, OL defines the canonical fork, so prefer OL's state.
        safe_block_hash
    };

    let finalized_block_hash =
        if finalized_block_hash.is_zero() || reader.is_in_canonical_chain(finalized_block_hash)? {
            finalized_block_hash
        } else {
            // Reth rejects forkchoice updates with a non-canonical finalized hash as
            // `invalid forkchoice state`. Drop the finalized pointer until this node has
            // canonicalized the same block locally.
            B256::ZERO
        };

    Ok(ForkchoiceState {
        head_block_hash,
        safe_block_hash,
        finalized_block_hash,
    })
}

fn forkchoice_state_from_consensus<N: NodeTypesWithDB + ProviderNodeTypes>(
    consensus_state: &ConsensusHeads,
    head_block_hash: B256,
    provider: &BlockchainProvider<N>,
) -> ProviderResult<ForkchoiceState> {
    forkchoice_state_from_consensus_with_reader(
        consensus_state,
        head_block_hash,
        &RethCanonicalChainReader::new(provider),
    )
}

/// Takes chain updates from OL and sequencer/p2p and updates the chain in engine (reth).
async fn engine_control_task_inner<N: NodeTypesWithDB + ProviderNodeTypes, E: ExecutionEngine>(
    mut preconf_rx: watch::Receiver<BlockNumHash>,
    mut consensus_rx: watch::Receiver<ConsensusHeads>,
    provider: BlockchainProvider<N>,
    engine: E,
) {
    let mut head_block_hash = provider
        .canonical_in_memory_state()
        .get_canonical_head()
        .hash();

    loop {
        select! {
            res = consensus_rx.changed() => {
                if res.is_err() {
                    // tx dropped; exit task
                    warn!("consensus_rx channel closed; exiting");
                    return;
                }
                // got a consensus update from ol
                let consensus_state = consensus_rx.borrow_and_update().clone();
                let update = match forkchoice_state_from_consensus(&consensus_state, head_block_hash, &provider) {
                    Ok(update) => update,
                    Err(err) => {
                        error!(?err, "failed to access blockchain provider");
                        continue;
                    }
                };

                if let Err(err) = engine.update_consensus_state(update).await {
                    warn!(?err, "forkchoice_update failed");
                    continue;
                }
            }
            res = preconf_rx.changed() => {
                if res.is_err() {
                    // tx dropped; exit task
                    warn!("preconf_rx channel closed; exiting");
                    return;
                }
                // got head block from sequencer / p2p
                let blocknumhash = *preconf_rx.borrow_and_update();
                let next_head_block_hash = B256::from_slice(blocknumhash.hash().as_slice());

                let update = ForkchoiceState {
                    head_block_hash: next_head_block_hash,
                    safe_block_hash: B256::ZERO,
                    finalized_block_hash: B256::ZERO,
                };
                if let Err(err) = engine.update_consensus_state(update).await {
                    warn!(?err, "forkchoice_update failed");
                    continue;
                }
                head_block_hash = next_head_block_hash;
            }
        }
    }
}

/// Creates an engine control task that processes chain updates from OL and sequencer.
pub fn create_engine_control_task<N: NodeTypesWithDB + ProviderNodeTypes, E: ExecutionEngine>(
    preconf_rx: watch::Receiver<BlockNumHash>,
    consensus_rx: watch::Receiver<ConsensusHeads>,
    provider: BlockchainProvider<N>,
    engine_control: E,
) -> impl Future<Output = ()> {
    engine_control_task_inner(preconf_rx, consensus_rx, provider, engine_control)
}

#[cfg(test)]
mod tests {
    use alloy_primitives::B256;
    use mockall::predicate::eq;
    use strata_acct_types::Hash;
    use strata_identifiers::Epoch;

    use super::*;

    fn hash_from_u8(value: u8) -> Hash {
        let mut bytes = [0u8; 32];
        bytes[0] = value;
        Hash::from(bytes)
    }

    fn b256_from_u8(value: u8) -> B256 {
        B256::from_slice(hash_from_u8(value).as_ref())
    }

    fn consensus_heads(confirmed: u8, finalized: u8) -> ConsensusHeads {
        ConsensusHeads {
            confirmed: hash_from_u8(confirmed),
            confirmed_epoch: Epoch::from(10u32),
            finalized: hash_from_u8(finalized),
            finalized_epoch: Epoch::from(9u32),
        }
    }

    #[test]
    fn forkchoice_keeps_head_when_confirmed_is_canonical() {
        let mut reader = MockCanonicalChainReader::new();
        let heads = consensus_heads(1, 2);
        let current_head = b256_from_u8(9);

        reader
            .expect_is_in_canonical_chain()
            .once()
            .with(eq(b256_from_u8(1)))
            .return_once(|_| Ok(true));
        reader
            .expect_is_in_canonical_chain()
            .once()
            .with(eq(b256_from_u8(2)))
            .return_once(|_| Ok(true));

        let state =
            forkchoice_state_from_consensus_with_reader(&heads, current_head, &reader).unwrap();

        assert_eq!(state.head_block_hash, current_head);
        assert_eq!(state.safe_block_hash, b256_from_u8(1));
        assert_eq!(state.finalized_block_hash, b256_from_u8(2));
    }

    #[test]
    fn forkchoice_rewrites_head_when_confirmed_is_noncanonical() {
        let mut reader = MockCanonicalChainReader::new();
        let heads = consensus_heads(3, 4);
        let current_head = b256_from_u8(9);

        reader
            .expect_is_in_canonical_chain()
            .once()
            .with(eq(b256_from_u8(3)))
            .return_once(|_| Ok(false));
        reader
            .expect_is_in_canonical_chain()
            .once()
            .with(eq(b256_from_u8(4)))
            .return_once(|_| Ok(true));

        let state =
            forkchoice_state_from_consensus_with_reader(&heads, current_head, &reader).unwrap();

        assert_eq!(state.head_block_hash, b256_from_u8(3));
        assert_eq!(state.safe_block_hash, b256_from_u8(3));
        assert_eq!(state.finalized_block_hash, b256_from_u8(4));
    }

    #[test]
    fn forkchoice_drops_noncanonical_finalized_hash() {
        let mut reader = MockCanonicalChainReader::new();
        let heads = consensus_heads(5, 6);
        let current_head = b256_from_u8(9);

        reader
            .expect_is_in_canonical_chain()
            .once()
            .with(eq(b256_from_u8(5)))
            .return_once(|_| Ok(true));
        reader
            .expect_is_in_canonical_chain()
            .once()
            .with(eq(b256_from_u8(6)))
            .return_once(|_| Ok(false));

        let state =
            forkchoice_state_from_consensus_with_reader(&heads, current_head, &reader).unwrap();

        assert_eq!(state.head_block_hash, current_head);
        assert_eq!(state.safe_block_hash, b256_from_u8(5));
        assert_eq!(state.finalized_block_hash, B256::ZERO);
    }

    #[test]
    fn forkchoice_skips_finalized_lookup_when_finalized_is_zero() {
        let mut reader = MockCanonicalChainReader::new();
        let heads = consensus_heads(7, 0);
        let current_head = b256_from_u8(9);

        reader
            .expect_is_in_canonical_chain()
            .once()
            .with(eq(b256_from_u8(7)))
            .return_once(|_| Ok(true));

        let state =
            forkchoice_state_from_consensus_with_reader(&heads, current_head, &reader).unwrap();

        assert_eq!(state.head_block_hash, current_head);
        assert_eq!(state.safe_block_hash, b256_from_u8(7));
        assert_eq!(state.finalized_block_hash, B256::ZERO);
    }
}
