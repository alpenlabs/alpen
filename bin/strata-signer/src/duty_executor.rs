//! Receives signing duties, deduplicates, signs, and submits results via RPC.

use std::{collections::HashSet, sync::Arc};

use jsonrpsee::core::ClientError;
use strata_common::ws_client::ManagedWsClient;
use strata_ol_rpc_api::OLSequencerRpcClient;
use strata_ol_sequencer::{
    BlockCompletionData, BlockSigningDuty, CheckpointSigningDuty, Duty, sign_checkpoint,
    sign_header,
};
use strata_primitives::{HexBytes64, buf::Buf32};
use thiserror::Error;
use tokio::{select, sync::mpsc, time};
use tracing::{debug, error, info, warn};

#[derive(Debug, Error)]
enum DutyExecError {
    #[error("failed completing block template: {0}")]
    CompleteTemplate(#[source] ClientError),

    #[error("failed submitting checkpoint signature: {0}")]
    CompleteCheckpoint(#[source] ClientError),
}

/// Receives duties from the fetcher, deduplicates them, signs, and submits via RPC.
pub(crate) async fn duty_executor_worker(
    rpc: Arc<ManagedWsClient>,
    mut duty_rx: mpsc::Receiver<Duty>,
    sequencer_key: Buf32,
) -> anyhow::Result<()> {
    let mut seen_duties: HashSet<Buf32> = HashSet::new();
    let (failed_tx, mut failed_rx) = mpsc::channel::<Buf32>(8);

    loop {
        select! {
            duty = duty_rx.recv() => {
                let Some(duty) = duty else {
                    return Ok(());
                };

                let duty_id = duty.generate_id();
                if seen_duties.contains(&duty_id) {
                    debug!(%duty_id, "skipping already seen duty");
                    continue;
                }
                seen_duties.insert(duty_id);

                tokio::spawn(handle_duty(
                    rpc.clone(),
                    duty,
                    sequencer_key,
                    failed_tx.clone(),
                ));
            }
            failed = failed_rx.recv() => {
                if let Some(duty_id) = failed {
                    warn!(%duty_id, "removing failed duty for retry");
                    seen_duties.remove(&duty_id);
                }
            }
        }
    }
}

async fn handle_duty(
    rpc: Arc<ManagedWsClient>,
    duty: Duty,
    sk: Buf32,
    failed_tx: mpsc::Sender<Buf32>,
) {
    let duty_id = duty.generate_id();
    debug!(%duty_id, %duty, "handling duty");

    let result = match duty {
        Duty::SignBlock(block_duty) => handle_sign_block(&rpc, block_duty, &sk).await,
        Duty::SignCheckpoint(cp_duty) => handle_sign_checkpoint(&rpc, cp_duty, &sk).await,
    };

    if let Err(err) = result {
        error!(%duty_id, %err, "duty failed");
        let _ = failed_tx.send(duty_id).await;
    }
}

async fn handle_sign_block(
    rpc: &ManagedWsClient,
    duty: BlockSigningDuty,
    sk: &Buf32,
) -> Result<(), DutyExecError> {
    // TODO: recheck this logic
    if let Some(wait) = duty.wait_duration() {
        debug!(wait_ms = %wait.as_millis(), "waiting for block target time");
        time::sleep(wait).await;
    }

    let template_id = duty.template_id();
    let sig = sign_header(duty.template.header(), sk);
    let completion = BlockCompletionData::from_signature(sig);

    rpc.complete_block_template(template_id, completion)
        .await
        .map_err(DutyExecError::CompleteTemplate)?;

    info!(%template_id, "block signed and submitted");
    Ok(())
}

async fn handle_sign_checkpoint(
    rpc: &ManagedWsClient,
    duty: CheckpointSigningDuty,
    sk: &Buf32,
) -> Result<(), DutyExecError> {
    let epoch = duty.epoch();
    let sig = sign_checkpoint(duty.checkpoint(), sk);

    debug!(%epoch, %sig, "signed checkpoint");

    rpc.complete_checkpoint_signature(epoch, HexBytes64(sig.0))
        .await
        .map_err(DutyExecError::CompleteCheckpoint)?;

    info!(%epoch, "checkpoint signature submitted");
    Ok(())
}
