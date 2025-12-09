use std::sync::Arc;

use alpen_ee_common::{
    get_inbox_messages_checked, ExecBlockStorage, OLBlockData, OLFinalizedStatus, SequencerOLClient,
};
use eyre::eyre;
use strata_identifiers::OLBlockCommitment;
use strata_snark_acct_types::MessageEntry;
use tokio::{
    select,
    sync::{mpsc, oneshot, watch},
};
use tracing::{error, warn};

use super::state::OLChainTrackerState;

pub(crate) enum OLChainTrackerQuery {
    GetFinalizedBlock(oneshot::Sender<OLBlockCommitment>),
    GetInboxMessages {
        from_slot: u64,
        to_slot: u64,
        response_tx: oneshot::Sender<eyre::Result<Vec<MessageEntry>>>,
    },
}

pub(crate) async fn ol_chain_tracker_task<
    TClient: SequencerOLClient,
    TStorage: ExecBlockStorage,
>(
    mut chainstatus_rx: watch::Receiver<OLFinalizedStatus>,
    mut query_rx: mpsc::Receiver<OLChainTrackerQuery>,
    mut state: OLChainTrackerState,
    client: Arc<TClient>,
    storage: Arc<TStorage>,
) {
    loop {
        select! {
            chainstatus_changed = chainstatus_rx.changed() => {
                if chainstatus_changed.is_err() {
                    warn!("channel is closed; shutting down");
                    break;
                }
                // we only track inbox messages from finalized blocks to include in block assembly.
                let ol_status = *chainstatus_rx.borrow_and_update();
                handle_chain_update(ol_status, &mut state, client.as_ref(), storage.as_ref()).await;
            }
            maybe_query = query_rx.recv() => {
                let Some(query) = maybe_query else {
                    warn!("channel is closed; shutting down");
                    break;
                };
                handle_chain_query(&state, query);
            }
        }
    }
}

fn handle_chain_query(state: &OLChainTrackerState, query: OLChainTrackerQuery) {
    match query {
        OLChainTrackerQuery::GetFinalizedBlock(tx) => {
            let _ = tx.send(state.best_block());
        }
        OLChainTrackerQuery::GetInboxMessages {
            from_slot,
            to_slot,
            response_tx,
        } => {
            let _ = response_tx.send(state.get_inbox_messages(from_slot, to_slot));
        }
    }
}

async fn handle_chain_update(
    ol_status: OLFinalizedStatus,
    state: &mut OLChainTrackerState,
    client: &impl SequencerOLClient,
    storage: &impl ExecBlockStorage,
) {
    // compare latest finalized block with local chain segment using db. get extend, revert info
    match track_ol_state(state, ol_status, client).await {
        Ok(TrackAction::Extend(ol_blocks)) => {
            // update state
            for OLBlockData {
                commitment,
                inbox_messages,
            } in ol_blocks
            {
                state.append_block(commitment, inbox_messages).unwrap();
            }

            if let Err(err) = handle_state_pruning(state, ol_status, storage).await {
                error!(?err, "failed to prune state");
            }
        }
        Ok(TrackAction::Reorg(_next)) => {
            // kill task and trigger app shutdown through TaskManager.
            panic!("Deep reorg detected. Manual resolution required.")
        }
        Err(err) => {
            error!("failed to track ol state; {}", err);
            // retry next cycle
            // TODO: unrecoverable error
        }
    };
}

enum TrackAction {
    Extend(Vec<OLBlockData>),
    Reorg(OLBlockCommitment),
}

async fn track_ol_state(
    state: &OLChainTrackerState,
    ol_status: OLFinalizedStatus,
    ol_client: &impl SequencerOLClient,
) -> eyre::Result<TrackAction> {
    let best_ol_block = state.best_block();
    // We only care about finalized ol blocks to use as inputs to block assembly.
    let remote_finalized_ol_block = ol_status.ol_block;

    if remote_finalized_ol_block == best_ol_block {
        // nothing to do
        return Ok(TrackAction::Extend(vec![]));
    }
    if remote_finalized_ol_block.slot() <= best_ol_block.slot() {
        warn!(
            local = ?best_ol_block,
            remote = ?remote_finalized_ol_block,
            "local finalized OL block ahead of OL"
        );

        return Ok(TrackAction::Reorg(remote_finalized_ol_block));
    }
    if remote_finalized_ol_block.slot() > best_ol_block.slot() {
        let blocks = get_inbox_messages_checked(
            ol_client,
            best_ol_block.slot(),
            remote_finalized_ol_block.slot(),
        )
        .await?;

        let (block_at_finalized_height, blocks) = {
            let mut iter = blocks.into_iter();
            let first = iter.next().expect("checked");

            (first, iter)
        };

        if block_at_finalized_height.commitment != best_ol_block {
            // The block we know to be finalized locally is not present in the OL chain.
            // OL chain has seen a deep reorg.
            // Avoid corrupting local data and exit to await manual resolution.

            warn!(
                local = ?best_ol_block,
                remote = ?block_at_finalized_height.commitment,
                "local finalized OL block not present in OL"
            );

            return Ok(TrackAction::Reorg(block_at_finalized_height.commitment));
        }

        return Ok(TrackAction::Extend(blocks.collect()));
    }

    unreachable!("all valid cases should have been handled above");
}

async fn handle_state_pruning(
    state: &mut OLChainTrackerState,
    finalized_status: OLFinalizedStatus,
    storage: &impl ExecBlockStorage,
) -> eyre::Result<()> {
    let finalized_ee_block = finalized_status.last_ee_block;
    // find last ol block whose data was included in this ee block
    let exec_package = storage
        .get_exec_block(finalized_ee_block)
        .await?
        .ok_or(eyre!(
            "finalized exec block not found: {finalized_ee_block:?}"
        ))?;

    let included_ol_block = exec_package.ol_block();
    state.prune_blocks(*included_ol_block)
}
