use std::future::Future;

use alloy_primitives::B256;
use alloy_rpc_types_engine::ForkchoiceState;
use alpen_reth_node::{AlpenBuiltPayload, AlpenEngineTypes};
use reth_node_builder::{
    BeaconConsensusEngineHandle, BuiltPayload, EngineApiMessageVersion, NodeTypesWithDB,
    PayloadTypes,
};
use reth_provider::{
    providers::{BlockchainProvider, ProviderNodeTypes},
    BlockHashReader, BlockNumReader, ProviderResult,
};
use strata_acct_types::Hash;
use tokio::{
    select,
    sync::{broadcast, watch},
};
use tracing::{error, warn};

use crate::{
    ol_tracker::ConsensusHeads,
    traits::{engine::ExecutionEngine, error::ExecutionEngineError},
};

#[derive(Debug, Clone)]
pub(crate) struct AlpenRethExecEngine {
    beacon_engine_handle: BeaconConsensusEngineHandle<AlpenEngineTypes>,
}

impl AlpenRethExecEngine {
    pub(crate) fn new(beacon_engine_handle: BeaconConsensusEngineHandle<AlpenEngineTypes>) -> Self {
        Self {
            beacon_engine_handle,
        }
    }
}

impl ExecutionEngine<AlpenBuiltPayload> for AlpenRethExecEngine {
    async fn submit_payload(&self, payload: AlpenBuiltPayload) -> Result<(), ExecutionEngineError> {
        self.beacon_engine_handle
            .new_payload(AlpenEngineTypes::block_to_payload(
                payload.block().to_owned(),
            ))
            .await
            .map(|_| ())
            .map_err(|e| ExecutionEngineError::payload_submission(e.to_string()))
    }

    async fn update_consenesus_state(
        &self,
        state: ForkchoiceState,
    ) -> Result<(), ExecutionEngineError> {
        self.beacon_engine_handle
            .fork_choice_updated(state, None, EngineApiMessageVersion::V4)
            .await
            .map(|_| ())
            .map_err(|e| ExecutionEngineError::fork_choice_update(e.to_string()))
    }
}

/// Check if `blockhash` is in canonical chain provided by [`BlockchainProvider`].
fn is_in_canonical_chain<N: NodeTypesWithDB + ProviderNodeTypes>(
    blockhash: B256,
    provider: &BlockchainProvider<N>,
) -> ProviderResult<bool> {
    let Some(block_number) = provider.block_number(blockhash)? else {
        return Ok(false);
    };
    let Some(canonical_blockhash) = provider.block_hash(block_number)? else {
        return Ok(false);
    };
    Ok(blockhash == canonical_blockhash)
}

fn forkchoice_state_from_consensus<N: NodeTypesWithDB + ProviderNodeTypes>(
    consensus_state: &ConsensusHeads,
    head_block_hash: B256,
    provider: &BlockchainProvider<N>,
) -> ProviderResult<ForkchoiceState> {
    let safe_block_hash = B256::from_slice(consensus_state.confirmed());
    let finalized_block_hash = B256::from_slice(consensus_state.finalized());

    let head_block_hash = if is_in_canonical_chain(safe_block_hash, provider)? {
        head_block_hash
    } else {
        // Safe block is not in canonical chain on reth.
        // This means either:
        // 1. This is during initial sync and OL chain is ahead of reth
        // 2. There is a fork
        // In either case, OL defines the canonical fork, so prefer OL's state.
        safe_block_hash
    };

    Ok(ForkchoiceState {
        head_block_hash,
        safe_block_hash,
        finalized_block_hash,
    })
}

/// Takes chain updates from OL and sequencer/p2p and updates the chain in engine (reth).
async fn engine_control_task_inner<
    N: NodeTypesWithDB + ProviderNodeTypes,
    E: ExecutionEngine<P>,
    P: Send,
>(
    mut preconf_rx: broadcast::Receiver<Hash>,
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
                        error!("failed to access blockchain provider: {:?}", err);
                        continue;
                    }
                };

                if let Err(err) = engine.update_consenesus_state(update).await {
                    warn!("forkchoice_update failed: {}", err);
                    continue;
                }
            }
            res = preconf_rx.recv() => {
                let next_head_block_hash = match res {
                    Ok(hash) => B256::from_slice(&hash),
                    Err(broadcast::error::RecvError::Closed) => {
                        // tx dropped; exit task
                        warn!("preconf_rx channel closed; exiting");
                        return;
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        warn!("preconf_rx channel lagged; ignoring");
                        continue;
                    }
                };

                // got head block from sequencer / p2p

                let update = ForkchoiceState {
                    head_block_hash: next_head_block_hash,
                    safe_block_hash: B256::ZERO,
                    finalized_block_hash: B256::ZERO,
                };
                if let Err(err) = engine.update_consenesus_state(update).await {
                    warn!("forkchoice_update failed: {}", err);
                    continue;
                }
                head_block_hash = next_head_block_hash;
            }
        }
    }
}

pub(crate) fn create_engine_control_task<
    N: NodeTypesWithDB + ProviderNodeTypes,
    E: ExecutionEngine<P>,
    P: Send,
>(
    preconf_rx: broadcast::Receiver<Hash>,
    consensus_rx: watch::Receiver<ConsensusHeads>,
    provider: BlockchainProvider<N>,
    engine_control: E,
) -> impl Future<Output = ()> {
    engine_control_task_inner(preconf_rx, consensus_rx, provider, engine_control)
}
