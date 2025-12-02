use std::sync::Arc;

use alpen_ee_common::{ConsensusHeads, ExecBlockRecord, ExecBlockStorage};
use eyre::eyre;
use strata_acct_types::Hash;
use tokio::sync::{mpsc, oneshot};
use tracing::error;

use crate::state::ExecChainState;

pub(crate) enum Query {
    GetBestBlock(oneshot::Sender<ExecBlockRecord>),
}

pub(crate) enum Message {
    /// query chain tracker state
    Query(Query),
    /// New exec block is available.
    /// The block data should be available through [`ExecBlockStorage`] when this event is
    /// processed.
    NewBlock(Hash),
    /// OL chain has updated.
    OLConsensusUpdate(ConsensusHeads),
}

pub(crate) async fn exec_chain_tracker_task<TStorage: ExecBlockStorage>(
    mut evt_rx: mpsc::Receiver<Message>,
    mut state: ExecChainState,
    mut preconf_head_tx: mpsc::Sender<Hash>,
    storage: Arc<TStorage>,
) {
    while let Some(evt) = evt_rx.recv().await {
        match evt {
            Message::Query(query) => handle_query(&mut state, query).await,
            Message::NewBlock(hash) => {
                if let Err(err) =
                    handle_new_block(&mut state, hash, storage.as_ref(), &mut preconf_head_tx).await
                {
                    error!("failed to handle new block; err = {err}");
                }
            }
            Message::OLConsensusUpdate(status) => {
                if let Err(err) = handle_ol_update(&mut state, status, storage.as_ref()).await {
                    error!("failed to handle OLConsensUpdate; err = {err}");
                }
            }
        }
    }
}

async fn handle_query(state: &mut ExecChainState, query: Query) {
    match query {
        Query::GetBestBlock(tx) => {
            let _ = tx.send(state.get_best_block().clone());
        }
    }
}

async fn handle_new_block<TStorage: ExecBlockStorage>(
    state: &mut ExecChainState,
    hash: Hash,
    storage: &TStorage,
    preconf_tx: &mut mpsc::Sender<Hash>,
) -> eyre::Result<()> {
    // get block from storage
    let record = storage
        .get_exec_block(hash)
        .await?
        .ok_or(eyre!("missing block: {:?}", hash))?;

    let prev_best = state.tip_blockhash();

    let best = state.append_block(record)?;

    if best != prev_best {
        // TODO: this channel being closed should be considered fatal
        preconf_tx.send(best).await?;
    }

    Ok(())
}

async fn handle_ol_update<TStorage: ExecBlockStorage>(
    state: &mut ExecChainState,
    status: ConsensusHeads,
    storage: &TStorage,
) -> eyre::Result<()> {
    // we only care about reorgs on the finalized state
    let finalized = *status.finalized();

    if finalized == state.finalized_blockhash() {
        // no need to do anything
        return Ok(());
    }

    if state.contains_unfinalized_block(&finalized) {
        // one of the unfinalized blocks got finalized.
        // update database
        storage.extend_finalized_chain(finalized).await?;

        // update in-memory state
        state.prune_finalized(finalized);

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
