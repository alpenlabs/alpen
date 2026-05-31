use std::sync::Arc;

use alpen_ee_common::{BlockNumHash, ConsensusHeads, ExecBlockStorage};
use alpen_ee_exec_chain::{
    ExecChainHandle, ExecChainMsg, ExecChainService, ExecChainServiceState, ExecChainState,
};
use strata_service::{AsyncExecutor, ServiceBuilder, TokioMpscInput};
use tokio::sync::watch;
use tracing::warn;

/// Starts the exec chain tracker as a service framework service.
///
/// Also spawns a consensus forwarder task that bridges the consensus watch
/// channel into the service's command channel.
pub(crate) async fn start_exec_chain_service<TStorage>(
    state: ExecChainState,
    preconf_head_tx: watch::Sender<BlockNumHash>,
    storage: Arc<TStorage>,
    mut consensus_watcher: watch::Receiver<ConsensusHeads>,
    executor: &impl AsyncExecutor,
) -> anyhow::Result<ExecChainHandle>
where
    TStorage: ExecBlockStorage + 'static,
{
    let service_state = ExecChainServiceState {
        chain_state: state,
        storage,
        preconf_head_tx,
    };

    let mut builder =
        ServiceBuilder::<ExecChainService<TStorage>, TokioMpscInput<ExecChainMsg>>::new()
            .with_state(service_state);

    let command_handle = builder.create_command_handle(64);
    let handle = ExecChainHandle::new(command_handle);

    builder.launch_async("exec_chain", executor).await?;

    let initial_update = consensus_watcher.borrow_and_update().clone();
    handle
        .new_consensus_state(initial_update)
        .await
        .map_err(|err| anyhow::anyhow!("failed to send initial consensus state: {err:?}"))?;

    // Spawn consensus forwarder: bridges watch channel -> command channel.
    let forwarder_handle = handle.clone();
    tokio::spawn(async move {
        loop {
            if consensus_watcher.changed().await.is_err() {
                warn!(target: "exec_chain_consensus_forwarder", "consensus_watch channel closed");
                break;
            }
            let update = consensus_watcher.borrow_and_update().clone();
            if forwarder_handle.new_consensus_state(update).await.is_err() {
                warn!(target: "exec_chain_consensus_forwarder", "exec_chain command channel closed");
                break;
            }
        }
    });

    Ok(handle)
}
