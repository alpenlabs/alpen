use std::sync::Arc;

use alpen_ee_common::{ConsensusHeads, ExecBlockRecord, ExecBlockStorage, StorageError};
use strata_acct_types::Hash;
use thiserror::Error;
use tokio::sync::{mpsc, oneshot, watch};
use tracing::error;

use crate::state::{ExecChainState, ExecChainStateError};

/// Errors that can occur during execution chain tracker operations.
#[derive(Debug, Error)]
pub(crate) enum ChainTrackerError {
    /// Preconf head channel is closed
    #[error("preconf head channel closed")]
    PreconfChannelClosed,
    /// Block not found in storage
    #[error("missing block: {0:?}")]
    MissingBlock(Hash),
    /// Storage error
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// Execution chain state error
    #[error(transparent)]
    ExecChainState(#[from] ExecChainStateError),
}

/// Queries for reading chain tracker state.
pub(crate) enum Query {
    GetBestBlock(oneshot::Sender<ExecBlockRecord>),
    IsCanonical(Hash, oneshot::Sender<bool>),
    GetFinalizedBlocknum(oneshot::Sender<u64>),
}

/// Channel receivers for the execution chain tracker task, split by priority.
pub(crate) struct TaskChannels {
    /// Highest priority: new block notifications
    pub new_block_rx: mpsc::Receiver<Hash>,
    /// Medium priority: OL consensus updates
    pub ol_update_rx: mpsc::Receiver<ConsensusHeads>,
    /// Lowest priority: queries
    pub query_rx: mpsc::Receiver<Query>,
}

/// Channel senders for the execution chain tracker task.
#[derive(Debug, Clone)]
pub(crate) struct TaskSenders {
    pub new_block_tx: mpsc::Sender<Hash>,
    pub ol_update_tx: mpsc::Sender<ConsensusHeads>,
    pub query_tx: mpsc::Sender<Query>,
}

/// Create a new set of task channels.
pub(crate) fn create_task_channels(buffer: usize) -> (TaskSenders, TaskChannels) {
    let (new_block_tx, new_block_rx) = mpsc::channel(buffer);
    let (ol_update_tx, ol_update_rx) = mpsc::channel(buffer);
    let (query_tx, query_rx) = mpsc::channel(buffer);

    (
        TaskSenders {
            new_block_tx,
            ol_update_tx,
            query_tx,
        },
        TaskChannels {
            new_block_rx,
            ol_update_rx,
            query_rx,
        },
    )
}

/// Main task loop for the execution chain tracker.
///
/// Processes incoming messages to update chain state, handle new blocks, and respond to queries.
/// The task exits if the preconf head channel is closed, as this is considered a fatal error.
///
/// Message priority (highest to lowest): `NewBlock` -> `OLConsensusUpdate` -> `Query`
pub(crate) async fn exec_chain_tracker_task<TStorage: ExecBlockStorage>(
    channels: TaskChannels,
    mut state: ExecChainState,
    preconf_head_tx: watch::Sender<Hash>,
    storage: Arc<TStorage>,
) {
    let TaskChannels {
        mut new_block_rx,
        mut ol_update_rx,
        mut query_rx,
    } = channels;

    loop {
        // biased ensures priority ordering: NewBlock > OLConsensusUpdate > Query
        tokio::select! {
            biased;

            Some(hash) = new_block_rx.recv() => {
                match handle_new_block(&mut state, hash, storage.as_ref(), &preconf_head_tx).await {
                    Err(ChainTrackerError::PreconfChannelClosed) => {
                        error!("preconf head channel closed, exiting task");
                        return;
                    }
                    Err(err) => {
                        error!(?err, "failed to handle new block");
                    }
                    Ok(()) => {}
                }
            }
            Some(status) = ol_update_rx.recv() => {
                match handle_ol_update(&mut state, status, storage.as_ref(), &preconf_head_tx).await {
                    Err(ChainTrackerError::PreconfChannelClosed) => {
                        error!("preconf head channel closed, exiting task");
                        return;
                    }
                    Err(err) => {
                        error!(?err, "failed to handle OLConsensUpdate");
                    }
                    Ok(()) => {}
                }
            }
            Some(query) = query_rx.recv() => {
                handle_query(&mut state, query).await;
            }
            else => break, // All channels closed
        }
    }
}

/// Handles state queries from external callers.
async fn handle_query(state: &mut ExecChainState, query: Query) {
    match query {
        Query::GetBestBlock(tx) => {
            let _ = tx.send(state.get_best_block().clone());
        }
        Query::IsCanonical(hash, tx) => {
            let _ = tx.send(state.is_canonical(&hash));
        }
        Query::GetFinalizedBlocknum(tx) => {
            let _ = tx.send(state.finalized_blocknum());
        }
    }
}

/// Handles a new block notification by fetching it from storage and appending to chain state.
///
/// Sends a preconf head update if the best tip changes.
async fn handle_new_block<TStorage: ExecBlockStorage>(
    state: &mut ExecChainState,
    hash: Hash,
    storage: &TStorage,
    preconf_tx: &watch::Sender<Hash>,
) -> Result<(), ChainTrackerError> {
    // Get block from storage
    let record = storage
        .get_exec_block(hash)
        .await?
        .ok_or(ChainTrackerError::MissingBlock(hash))?;

    // Append to tracker state and emit best hash if changed
    let prev_best = state.tip_blockhash();
    let new_best = state.append_block(record)?;
    if new_best != prev_best {
        preconf_tx
            .send(new_best)
            .map_err(|_| ChainTrackerError::PreconfChannelClosed)?;
    }

    Ok(())
}

/// Handles an OL consensus update.
///
/// Updates finalized state if a tracked unfinalized block becomes finalized.
async fn handle_ol_update<TStorage: ExecBlockStorage>(
    state: &mut ExecChainState,
    status: ConsensusHeads,
    storage: &TStorage,
    preconf_tx: &watch::Sender<Hash>,
) -> Result<(), ChainTrackerError> {
    // we only care about reorgs on the finalized state
    let finalized = *status.finalized();

    if finalized == state.finalized_blockhash() {
        // no need to do anything
        return Ok(());
    }

    if state.contains_unfinalized_block(&finalized) {
        // one of the unfinalized blocks got finalized.
        // update database
        let prev_best = state.tip_blockhash();
        storage.extend_finalized_chain(finalized).await?;

        // update in-memory state
        state
            .prune_finalized(finalized)
            .expect("finalized exists in unfinalized blocks");
        let new_best = state.tip_blockhash();

        if prev_best != new_best {
            // finalization has triggered a reorg of the tip
            preconf_tx
                .send(new_best)
                .map_err(|_| ChainTrackerError::PreconfChannelClosed)?;
        }

        return Ok(());
    }

    if state.contains_orphan_block(&finalized) {
        // finalized block is a known but unconnected block
        // TODO: store the finalized state and retry later
        return Ok(());
    }

    // TODO: we have a deep reorg beyond what we consider finalized.
    unimplemented!("deep reorg");
}
