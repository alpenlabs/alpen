use std::{future::Future, sync::Arc};

use alpen_ee_common::{ConsensusHeads, ExecBlockRecord, ExecBlockStorage};
use strata_acct_types::Hash;
use tokio::sync::{mpsc, oneshot, watch};
use tracing::warn;

use crate::{
    state::ExecChainState,
    task::{exec_chain_tracker_task, Message, Query},
};

/// Handle for interacting with the execution chain tracker task.
///
/// Provides methods to query chain state and submit new blocks or consensus updates.
#[derive(Debug, Clone)]
pub struct ExecChainHandle {
    msg_tx: mpsc::Sender<Message>,
}

impl ExecChainHandle {
    /// Fetch the best canonical exec block.
    pub async fn get_best_block(&self) -> eyre::Result<ExecBlockRecord> {
        let (tx, rx) = oneshot::channel();

        self.msg_tx
            .send(Message::Query(Query::GetBestBlock(tx)))
            .await?;

        rx.await.map_err(Into::into)
    }

    /// Submit new exec block to be tracked.
    pub async fn new_block(&self, hash: Hash) -> eyre::Result<()> {
        self.msg_tx
            .send(Message::NewBlock(hash))
            .await
            .map_err(Into::into)
    }

    /// Submit new OL Consensus state.
    pub async fn new_consensus_state(&self, consensus: ConsensusHeads) -> eyre::Result<()> {
        self.msg_tx
            .send(Message::OLConsensusUpdate(consensus))
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
    let (msg_tx, msg_rx) = mpsc::channel(64);
    let task_fut = exec_chain_tracker_task(msg_rx, state, preconf_head_tx, storage);

    let handle = ExecChainHandle { msg_tx };

    (handle, task_fut)
}

/// Task to wire consensus watch channel and internal msg channel.
pub fn build_exec_chain_consensus_forwarder_task(
    handle: ExecChainHandle,
    mut consensus_watch: watch::Receiver<ConsensusHeads>,
) -> impl Future<Output = ()> {
    let tx = handle.msg_tx.clone();
    async move {
        loop {
            if consensus_watch.changed().await.is_err() {
                // channel is closed; exit this task
                warn!(target: "exec_chain_consensus_forwarder", "consensus_watch channel closed");
                break;
            }
            let update = consensus_watch.borrow_and_update().clone();
            if tx.send(Message::OLConsensusUpdate(update)).await.is_err() {
                warn!(target: "exec_chain_consensus_forwarder", "chain_exec msg channel closed");
                break;
            }
        }
    }
}
