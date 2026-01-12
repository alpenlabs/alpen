use std::{future::Future, sync::Arc};

use alpen_ee_common::{ConsensusHeads, ExecBlockRecord, ExecBlockStorage};
use strata_acct_types::Hash;
use tokio::sync::{oneshot, watch};
use tracing::warn;

use crate::{
    state::ExecChainState,
    task::{create_task_channels, exec_chain_tracker_task, Query, TaskSenders},
};

/// Handle for interacting with the execution chain tracker task.
///
/// Provides methods to query chain state and submit new blocks or consensus updates.
#[derive(Debug, Clone)]
pub struct ExecChainHandle {
    senders: TaskSenders,
}

impl ExecChainHandle {
    /// Fetch the best canonical exec block.
    pub async fn get_best_block(&self) -> eyre::Result<ExecBlockRecord> {
        let (tx, rx) = oneshot::channel();

        self.senders.query_tx.send(Query::GetBestBlock(tx)).await?;

        rx.await.map_err(Into::into)
    }

    /// Check if a block is on the canonical chain.
    ///
    /// Returns `true` if the block with the given hash lies on the path from
    /// the finalized block to the current best tip.
    pub async fn is_canonical(&self, hash: Hash) -> eyre::Result<bool> {
        let (tx, rx) = oneshot::channel();

        self.senders
            .query_tx
            .send(Query::IsCanonical(hash, tx))
            .await?;

        rx.await.map_err(Into::into)
    }

    /// Get the block number of the current finalized block.
    pub async fn get_finalized_blocknum(&self) -> eyre::Result<u64> {
        let (tx, rx) = oneshot::channel();

        self.senders
            .query_tx
            .send(Query::GetFinalizedBlocknum(tx))
            .await?;

        rx.await.map_err(Into::into)
    }

    /// Submit new exec block to be tracked.
    pub async fn new_block(&self, hash: Hash) -> eyre::Result<()> {
        self.senders
            .new_block_tx
            .send(hash)
            .await
            .map_err(Into::into)
    }

    /// Submit new OL Consensus state.
    pub async fn new_consensus_state(&self, consensus: ConsensusHeads) -> eyre::Result<()> {
        self.senders
            .ol_update_tx
            .send(consensus)
            .await
            .map_err(Into::into)
    }
}

/// Creates the execution chain tracker task and handle for interacting with it.
pub fn build_exec_chain_task<TStorage: ExecBlockStorage>(
    state: ExecChainState,
    preconf_head_tx: watch::Sender<Hash>,
    storage: Arc<TStorage>,
) -> (ExecChainHandle, impl Future<Output = ()>) {
    let (senders, channels) = create_task_channels(64);
    let task_fut = exec_chain_tracker_task(channels, state, preconf_head_tx, storage);

    let handle = ExecChainHandle { senders };

    (handle, task_fut)
}

/// Task to wire consensus watch channel and internal msg channel.
pub fn build_exec_chain_consensus_forwarder_task(
    handle: ExecChainHandle,
    mut consensus_watch: watch::Receiver<ConsensusHeads>,
) -> impl Future<Output = ()> {
    let tx = handle.senders.ol_update_tx.clone();
    async move {
        loop {
            if consensus_watch.changed().await.is_err() {
                // channel is closed; exit this task
                warn!(target: "exec_chain_consensus_forwarder", "consensus_watch channel closed");
                break;
            }
            let update = consensus_watch.borrow_and_update().clone();
            if tx.send(update).await.is_err() {
                warn!(target: "exec_chain_consensus_forwarder", "chain_exec msg channel closed");
                break;
            }
        }
    }
}
