use std::{future::Future, sync::Arc};

use alpen_ee_common::{ConsensusHeads, ExecBlockRecord, ExecBlockStorage};
use strata_acct_types::Hash;
use tokio::sync::{mpsc, oneshot};

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
pub fn build_task<TStorage: ExecBlockStorage>(
    state: ExecChainState,
    preconf_head_tx: mpsc::Sender<Hash>,
    storage: Arc<TStorage>,
) -> (ExecChainHandle, impl Future<Output = ()>) {
    let (msg_tx, msg_rx) = mpsc::channel(64);
    let task_fut = exec_chain_tracker_task(msg_rx, state, preconf_head_tx, storage);

    let handle = ExecChainHandle { msg_tx };

    (handle, task_fut)
}
