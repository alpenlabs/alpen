use std::{future::Future, sync::Arc};

use alpen_ee_common::{ExecBlockStorage, OLFinalizedStatus, SequencerOLClient};
use strata_identifiers::OLBlockCommitment;
use tokio::sync::{mpsc, oneshot, watch};

use super::task::OLChainTrackerQuery;
use crate::{
    ol_chain_tracker::{state::InboxMessages, task::ol_chain_tracker_task},
    OLChainTrackerState,
};

#[derive(Debug)]
pub struct OLChainTrackerHandle {
    query_tx: mpsc::Sender<OLChainTrackerQuery>,
}

impl OLChainTrackerHandle {
    pub async fn get_finalized_block(&self) -> eyre::Result<OLBlockCommitment> {
        let (tx, rx) = oneshot::channel();
        self.query_tx
            .send(OLChainTrackerQuery::GetFinalizedBlock(tx))
            .await?;
        rx.await.map_err(Into::into)
    }

    pub async fn get_inbox_messages(
        &self,
        from_slot: u64,
        to_slot: u64,
    ) -> eyre::Result<InboxMessages> {
        let (tx, rx) = oneshot::channel();
        self.query_tx
            .send(OLChainTrackerQuery::GetInboxMessages {
                from_slot,
                to_slot,
                response_tx: tx,
            })
            .await?;
        rx.await.map_err(eyre::Error::from)?
    }
}

pub fn build_ol_chain_tracker<TClient: SequencerOLClient, TStorage: ExecBlockStorage>(
    state: OLChainTrackerState,
    chainstatus_rx: watch::Receiver<OLFinalizedStatus>,
    client: Arc<TClient>,
    storage: Arc<TStorage>,
) -> (OLChainTrackerHandle, impl Future<Output = ()>) {
    let (query_tx, query_rx) = mpsc::channel(64);
    let handle = OLChainTrackerHandle { query_tx };
    let task = ol_chain_tracker_task(chainstatus_rx, query_rx, state, client, storage);

    (handle, task)
}
