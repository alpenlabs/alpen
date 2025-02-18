use std::{collections::HashSet, sync::Arc};

use strata_rpc_api::StrataSequencerApiClient;
use strata_rpc_types::HexBytes64;
use strata_sequencer::{
    block_template::{BlockCompletionData, BlockGenerationConfig},
    duty::types::{BlockSigningDuty, CheckpointDuty, Duty, DutyId, IdentityData},
    utils::now_millis,
};
use thiserror::Error;
use tokio::{runtime::Handle, select, sync::mpsc};
use tracing::{debug, error, info, warn};

use crate::helpers::{sign_checkpoint, sign_header};

#[derive(Debug, Error)]
enum DutyExecError {
    #[error("failed generating template: {0}")]
    GenerateTemplate(jsonrpsee::core::ClientError),
    #[error("failed completing template: {0}")]
    CompleteTemplate(jsonrpsee::core::ClientError),
    #[error("failed submitting checkpoint signature: {0}")]
    CompleteCheckpoint(jsonrpsee::core::ClientError),
}

pub(crate) async fn duty_executor_worker<R>(
    rpc: Arc<R>,
    mut duty_rx: mpsc::Receiver<Duty>,
    handle: Handle,
    idata: IdentityData,
) -> anyhow::Result<()>
where
    R: StrataSequencerApiClient + Send + Sync + 'static,
{
    // Keep track of seen duties to avoid processing the same duty multiple times.
    // Does not need to be persisted, as new duties are generated based on current chain state.
    let mut seen_duties = HashSet::new();
    let (failed_duties_tx, mut failed_duties_rx) = mpsc::channel::<DutyId>(8);

    loop {
        select! {
            duty = duty_rx.recv() => {
                if let Some(duty) = duty {
                    let duty_id = duty.id();
                    if seen_duties.contains(&duty_id) {
                        debug!(%duty_id, "skipping already seen duty");
                        continue;
                    }
                    seen_duties.insert(duty_id);
                    handle.spawn(handle_duty(rpc.clone(), duty, idata.clone(), failed_duties_tx.clone()));
                } else {
                    // tx is closed, we are done
                    return Ok(());
                }
            }
            failed_duty = failed_duties_rx.recv() => {
                if let Some(duty_id) = failed_duty {
                    // remove from seen duties, so we can retry if the duty is seen again
                    warn!(%duty_id, "removing failed duty");
                    seen_duties.remove(&duty_id);
                }
            }
        }
    }
}

async fn handle_duty<R>(
    rpc: Arc<R>,
    duty: Duty,
    idata: IdentityData,
    failed_duties_tx: mpsc::Sender<DutyId>,
) where
    R: StrataSequencerApiClient + Send + Sync,
{
    let duty_id = duty.id();
    debug!(%duty_id, ?duty, "handle_duty");
    let duty_result = match duty.clone() {
        Duty::SignBlock(duty) => handle_sign_block_duty(rpc, duty, duty_id, &idata).await,
        Duty::CommitBatch(duty) => handle_commit_batch_duty(rpc, duty, duty_id, &idata).await,
    };

    if let Err(error) = duty_result {
        error!(%duty_id, %error, "duty failed");
        let _ = failed_duties_tx.send(duty.id()).await;
    }
}

async fn handle_sign_block_duty<R>(
    rpc: Arc<R>,
    duty: BlockSigningDuty,
    duty_id: DutyId,
    idata: &IdentityData,
) -> Result<(), DutyExecError>
where
    R: StrataSequencerApiClient + Send + Sync,
{
    let now = now_millis();
    if now < duty.target_ts() {
        // wait until target time
        // TODO: ensure duration is within some bounds
        warn!(%duty_id, %now, target = duty.target_ts(), "got duty too early; sleeping till target time");
        tokio::time::sleep(tokio::time::Duration::from_millis(
            duty.target_ts() - now_millis(),
        ))
        .await;
    }

    // should this keep track of previously signed slots and dont sign conflicting blocks ?
    let template = rpc
        .get_block_template(BlockGenerationConfig::from_parent_block_id(duty.parent()))
        .await
        .map_err(DutyExecError::GenerateTemplate)?;

    let id = template.template_id();

    info!(%duty_id, block_id = %id, "got block template");

    let signature = sign_header(template.header(), &idata.key);
    let completion = BlockCompletionData::from_signature(signature);

    rpc.complete_block_template(template.template_id(), completion)
        .await
        .map_err(DutyExecError::CompleteTemplate)?;

    info!(%duty_id, block_id = %id, "block signing complete");

    Ok(())
}

async fn handle_commit_batch_duty<R>(
    rpc: Arc<R>,
    duty: CheckpointDuty,
    duty_id: DutyId,
    idata: &IdentityData,
) -> Result<(), DutyExecError>
where
    R: StrataSequencerApiClient + Send + Sync,
{
    let sig = sign_checkpoint(duty.inner(), &idata.key);

    debug!(%duty_id, %sig, "checkpoint signature");

    rpc.complete_checkpoint_signature(duty.inner().batch_info().epoch(), HexBytes64(sig.0))
        .await
        .map_err(DutyExecError::CompleteCheckpoint)?;

    Ok(())
}
