use std::{collections::HashSet, sync::Arc};

use jsonrpsee::core::ClientError;
use ssz::Decode;
use strata_checkpoint_types_ssz::CheckpointPayload;
use strata_crypto::hash;
use strata_identifiers::{Epoch, OLBlockCommitment};
use strata_ol_chain_types_new::OLBlockHeader;
use strata_primitives::HexBytes64;
use strata_rpc_api_new::OLSequencerRpcClient;
use strata_rpc_types_new::{
    RpcBlockGenerationConfig, RpcOLBlockSigningDuty, RpcOLBlockTemplate, RpcOLCheckpointDuty,
    RpcOLDuty,
};
use strata_sequencer_new::{
    duty::types::{DutyId, IdentityData},
    utils::now_millis,
};
use thiserror::Error;
use tokio::{runtime::Handle, select, sync::mpsc, time};
use tracing::{debug, error, info, warn};

use crate::helpers::{sign_checkpoint_payload, sign_ol_header};

#[derive(Debug, Error)]
enum DutyExecError {
    #[error("failed generating template: {0}")]
    GenerateTemplate(ClientError),

    #[error("failed completing template: {0}")]
    CompleteTemplate(ClientError),

    #[error("failed submitting checkpoint signature: {0}")]
    CompleteCheckpoint(ClientError),

    #[error("failed decoding checkpoint payload: {0}")]
    DecodeCheckpoint(String),

    #[error("failed decoding block header: {0}")]
    DecodeHeader(String),
}

pub(crate) async fn duty_executor_worker<R>(
    rpc: Arc<R>,
    mut duty_rx: mpsc::Receiver<RpcOLDuty>,
    handle: Handle,
    idata: IdentityData,
) -> anyhow::Result<()>
where
    R: OLSequencerRpcClient + Send + Sync + 'static,
{
    let mut seen_duties = HashSet::new();
    let (failed_duties_tx, mut failed_duties_rx) = mpsc::channel::<DutyId>(8);

    loop {
        select! {
            duty = duty_rx.recv() => {
                if let Some(duty) = duty {
                    let duty_id = duty_id(&duty);
                    if seen_duties.contains(&duty_id) {
                        debug!(%duty_id, "skipping already seen duty");
                        continue;
                    }
                    seen_duties.insert(duty_id);
                    handle.spawn(handle_duty(rpc.clone(), duty, idata.clone(), failed_duties_tx.clone()));
                } else {
                    return Ok(());
                }
            }
            failed_duty = failed_duties_rx.recv() => {
                if let Some(duty_id) = failed_duty {
                    warn!(%duty_id, "removing failed duty");
                    seen_duties.remove(&duty_id);
                }
            }
        }
    }
}

fn duty_id(duty: &RpcOLDuty) -> DutyId {
    match duty {
        RpcOLDuty::CommitBatch(duty) => {
            let epoch = duty.epoch();
            hash::raw(&epoch.to_be_bytes())
        }
        RpcOLDuty::SignBlock(duty) => {
            let mut buf = [0u8; 8 + 32 + 8];
            buf[..8].copy_from_slice(&duty.target_slot().to_be_bytes());
            buf[8..40].copy_from_slice(duty.parent().as_ref());
            buf[40..].copy_from_slice(&duty.target_ts().to_be_bytes());
            hash::raw(&buf)
        }
    }
}

async fn handle_duty<R>(
    rpc: Arc<R>,
    duty: RpcOLDuty,
    idata: IdentityData,
    failed_duties_tx: mpsc::Sender<DutyId>,
) where
    R: OLSequencerRpcClient + Send + Sync,
{
    let duty_id = duty_id(&duty);
    debug!(%duty_id, ?duty, "handle_duty");
    let duty_result = match duty.clone() {
        RpcOLDuty::SignBlock(duty) => handle_sign_block_duty(rpc, duty, duty_id, &idata).await,
        RpcOLDuty::CommitBatch(duty) => handle_commit_batch_duty(rpc, duty, duty_id, &idata).await,
    };

    if let Err(err) = duty_result {
        error!(%duty_id, %err, "duty failed");
        let _ = failed_duties_tx.send(duty_id).await;
    }
}

async fn handle_sign_block_duty<R>(
    rpc: Arc<R>,
    duty: RpcOLBlockSigningDuty,
    duty_id: DutyId,
    idata: &IdentityData,
) -> Result<(), DutyExecError>
where
    R: OLSequencerRpcClient + Send + Sync,
{
    let now = now_millis();
    if now < duty.target_ts() {
        warn!(%duty_id, %now, target = duty.target_ts(), "got duty too early; sleeping till target time");
        time::sleep(time::Duration::from_millis(duty.target_ts() - now)).await;
    }

    let parent_slot = duty.target_slot().saturating_sub(1);
    let parent_commitment = OLBlockCommitment::new(parent_slot, duty.parent());
    let config = RpcBlockGenerationConfig::new(parent_commitment);

    let template: RpcOLBlockTemplate = rpc
        .get_ol_block_template(config)
        .await
        .map_err(DutyExecError::GenerateTemplate)?;

    let header = OLBlockHeader::from_ssz_bytes(&template.header().0)
        .map_err(|e| DutyExecError::DecodeHeader(e.to_string()))?;

    info!(%duty_id, block_id = %template.template_id(), "got block template");

    let signature = sign_ol_header(&header, &idata.key);

    let sig = HexBytes64::from(signature.0);
    rpc.complete_ol_block_template(template.template_id(), sig)
        .await
        .map_err(DutyExecError::CompleteTemplate)?;

    info!(%duty_id, block_id = %template.template_id(), "block signing complete");

    Ok(())
}

async fn handle_commit_batch_duty<R>(
    rpc: Arc<R>,
    duty: RpcOLCheckpointDuty,
    duty_id: DutyId,
    idata: &IdentityData,
) -> Result<(), DutyExecError>
where
    R: OLSequencerRpcClient + Send + Sync,
{
    let payload = CheckpointPayload::from_ssz_bytes(&duty.checkpoint_payload().0)
        .map_err(|e| DutyExecError::DecodeCheckpoint(e.to_string()))?;
    let sig = sign_checkpoint_payload(&payload, &idata.key);
    let epoch: Epoch = duty.epoch();

    debug!(%epoch, %duty_id, %sig, "signed checkpoint");

    let sig = HexBytes64::from(sig.0);
    rpc.complete_checkpoint_signature(epoch as u64, sig)
        .await
        .map_err(DutyExecError::CompleteCheckpoint)?;

    Ok(())
}
