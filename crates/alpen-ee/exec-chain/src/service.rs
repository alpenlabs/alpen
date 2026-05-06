//! Service framework integration for the exec chain tracker.

use std::{fmt, marker::PhantomData, sync::Arc};

use alpen_ee_common::{BlockNumHash, ConsensusHeads, ExecBlockRecord, ExecBlockStorage};
use serde::Serialize;
use strata_acct_types::Hash;
use strata_service::{AsyncService, CommandCompletionSender, Response, Service, ServiceState};
use tokio::sync::watch;
use tracing::error;

use crate::{
    state::ExecChainState,
    task::{handle_new_block, handle_ol_update, ChainTrackerError},
};

/// Exec chain tracker service marker type.
#[derive(Debug)]
pub struct ExecChainService<TStorage>(PhantomData<TStorage>);

/// Minimal status for the service framework.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ExecChainStatus;

/// Unified message type for the exec chain service.
#[derive(Debug)]
pub enum ExecChainMsg {
    /// Notify about a new execution block (must exist in storage).
    NewBlock(Hash),

    /// Submit an OL consensus state update.
    OLUpdate(ConsensusHeads),

    /// Query the best canonical exec block.
    GetBestBlock(CommandCompletionSender<ExecBlockRecord>),

    /// Check if a block is on the canonical chain.
    IsCanonical(Hash, CommandCompletionSender<bool>),

    /// Get the block number of the current finalized block.
    GetFinalizedBlocknum(CommandCompletionSender<u64>),
}

/// Service state for the exec chain tracker.
pub struct ExecChainServiceState<TStorage> {
    pub chain_state: ExecChainState,
    pub storage: Arc<TStorage>,
    pub preconf_head_tx: watch::Sender<BlockNumHash>,
}

impl<TStorage> fmt::Debug for ExecChainServiceState<TStorage> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecChainServiceState")
            .field("chain_state", &self.chain_state)
            .finish_non_exhaustive()
    }
}

impl<TStorage> ServiceState for ExecChainServiceState<TStorage>
where
    TStorage: ExecBlockStorage + 'static,
{
    fn name(&self) -> &str {
        "exec_chain"
    }

    fn span_prefix(&self) -> &str {
        "exec_chain"
    }
}

impl<TStorage> Service for ExecChainService<TStorage>
where
    TStorage: ExecBlockStorage + 'static,
{
    type State = ExecChainServiceState<TStorage>;
    type Msg = ExecChainMsg;
    type Status = ExecChainStatus;

    fn get_status(_state: &Self::State) -> Self::Status {
        ExecChainStatus
    }
}

impl<TStorage> AsyncService for ExecChainService<TStorage>
where
    TStorage: ExecBlockStorage + 'static,
{
    async fn process_input(
        state: &mut Self::State,
        input: ExecChainMsg,
    ) -> anyhow::Result<Response> {
        match input {
            ExecChainMsg::NewBlock(hash) => {
                match handle_new_block(
                    &mut state.chain_state,
                    hash,
                    state.storage.as_ref(),
                    &state.preconf_head_tx,
                )
                .await
                {
                    Err(ChainTrackerError::PreconfChannelClosed) => {
                        return Err(anyhow::anyhow!("preconf head channel closed"));
                    }
                    Err(err) => {
                        error!(?err, "failed to handle new block");
                    }
                    Ok(()) => {}
                }
            }
            ExecChainMsg::OLUpdate(status) => {
                match handle_ol_update(
                    &mut state.chain_state,
                    status,
                    state.storage.as_ref(),
                    &state.preconf_head_tx,
                )
                .await
                {
                    Err(ChainTrackerError::PreconfChannelClosed) => {
                        return Err(anyhow::anyhow!("preconf head channel closed"));
                    }
                    Err(err) => {
                        error!(?err, "failed to handle OL consensus update");
                    }
                    Ok(()) => {}
                }
            }
            ExecChainMsg::GetBestBlock(completion) => {
                completion
                    .send(state.chain_state.get_best_block().clone())
                    .await;
            }
            ExecChainMsg::IsCanonical(hash, completion) => {
                completion.send(state.chain_state.is_canonical(&hash)).await;
            }
            ExecChainMsg::GetFinalizedBlocknum(completion) => {
                completion
                    .send(state.chain_state.finalized_blocknum())
                    .await;
            }
        }

        Ok(Response::Continue)
    }
}
